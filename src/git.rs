use crate::error::*;
use clap::crate_name;
use dirs::home_dir;
use git2::{Cred, RemoteCallbacks};
use log::*;
use std::{io::Write, path::Path};
use tokio::{
    fs::File,
    io::{AsyncReadExt, BufReader},
};

pub async fn handle_heavy_paths(
    paths: &[&Path],
    gist_id: &str,
    need_temp_file: bool,
) -> Result<()> {
    let tmp_dir = tempfile::Builder::new()
        .prefix(concat!(crate_name!(), "-"))
        .tempdir()?;

    let result = handle_heavy_paths_inner(paths, gist_id, tmp_dir.path(), need_temp_file).await;

    debug!("tmp_dir.close()");
    if let Err(e) = tmp_dir.close() {
        error!("couldn't delete tempdir: {}", e);
    }

    result?;

    Ok(())
}

fn get_callbacks() -> RemoteCallbacks<'static> {
    // Prepare callbacks.
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(|_url, username_from_url, _allowed_types| {
        Cred::ssh_key(
            username_from_url.unwrap(),
            None,
            &home_dir()
                .chain_err(|| "home_dir()")
                .unwrap()
                .join(".ssh")
                .join("id_rsa"),
            None,
        )
    });
    callbacks
}

async fn handle_heavy_paths_inner(
    paths: &[&Path],
    gist_id: &str,
    dir: &Path,
    need_temp_file: bool,
) -> Result<()> {
    // Prepare fetch options.
    let mut fo = git2::FetchOptions::new();
    fo.remote_callbacks(get_callbacks());

    // Prepare builder.
    let mut builder = git2::build::RepoBuilder::new();
    builder.fetch_options(fo);

    info!("cloning git repo to push {} heavy files", paths.len());
    debug!("cloning in {}", dir.display());
    // Clone the project.
    let repo = builder
        .bare(true)
        .clone(&format!("git@gist.github.com:{}.git", gist_id), dir)?;

    debug!("find_branch");
    let branch = repo.find_branch("origin/master", git2::BranchType::Remote)?;
    let commit = branch.get().peel_to_commit().unwrap();
    let tree = branch.get().peel_to_tree().unwrap();
    debug!("treebuilder");
    let mut tree_builder = repo.treebuilder(Some(&tree))?;

    if need_temp_file {
        debug!("removing temp");
        tree_builder.remove("temp")?;
    }

    for path in paths {
        let odb = repo.odb()?;

        let file = File::open(path).await?;
        let mut odb_writer =
            odb.writer(file.metadata().await?.len() as _, git2::ObjectType::Blob)?;

        let mut reader = BufReader::new(file);

        let mut buf = [0u8; 8 * 1024];
        loop {
            let len = reader.read(&mut buf).await?;
            if len == 0 {
                break;
            } else {
                odb_writer.write_all(&buf[..len])?;
            }
        }

        let oid = odb_writer.finalize()?;
        let filename = path.file_name().unwrap();

        debug!("git add {:?}", path);
        tree_builder.insert(filename, oid, 0o100644)?;
    }

    let new_tree_oid = tree_builder.write()?;
    debug!("tree built");
    let new_tree = repo.find_tree(new_tree_oid)?;

    debug!("committing");
    // create dangling commit with fresh, new tree
    let new_commit = repo.commit(
        None,
        &commit.author(),
        &commit.committer(),
        "",
        &new_tree,
        &[],
    )?;
    debug!("committed {}", new_commit);

    debug!("resetting to new tree");
    repo.reset(
        &repo.find_commit(new_commit)?.into_object(),
        git2::ResetType::Soft,
        None,
    )?;

    let mut remote = repo.find_remote("origin")?;

    // Prepare fetch options.
    let mut po = git2::PushOptions::new();
    po.remote_callbacks(get_callbacks());

    debug!("pushing");
    remote.push(&["+refs/heads/master:refs/heads/master"], Some(&mut po))?;

    Ok(())
}
