use crate::error::*;
use clap::crate_name;
use dirs::home_dir;
use git2::{
    build::RepoBuilder, BranchType, Cred, FetchOptions, ObjectType, PushOptions, RemoteCallbacks,
    ResetType,
};
use log::*;
use std::{fs::File, io, io::BufReader, path::PathBuf};

pub async fn handle_heavy_paths(
    paths: Vec<PathBuf>,
    gist_id: String,
    need_temp_file: bool,
) -> Result<()> {
    let tmp_dir = tempfile::Builder::new()
        .prefix(concat!(crate_name!(), "-"))
        .tempdir()?;

    let tmp_dir_path = tmp_dir.path().to_path_buf();

    let result = tokio::task::spawn_blocking(move || {
        handle_heavy_paths_inner(paths, gist_id, tmp_dir_path, need_temp_file)
    })
    .await?;

    debug!("tmp_dir.close()");
    if let Err(e) = tmp_dir.close() {
        error!("couldn't delete tempdir: {}", e);
    }

    result?;

    Ok(())
}

fn get_callbacks(pushing: bool) -> RemoteCallbacks<'static> {
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
    if pushing {
        // this doesn't seem to work but it would be nice!
        callbacks.transfer_progress(|progress| {
            info!(
                "{}/{} objects received, {} bytes",
                progress.received_objects(),
                progress.total_objects(),
                progress.received_bytes()
            );
            true
        });
        callbacks.sideband_progress(|data| {
            info!("{}", String::from_utf8_lossy(data));
            true
        });
    }
    callbacks
}

fn handle_heavy_paths_inner(
    paths: Vec<PathBuf>,
    gist_id: String,
    dir: PathBuf,
    need_temp_file: bool,
) -> Result<()> {
    // Prepare fetch options.
    let mut fo = FetchOptions::new();
    fo.remote_callbacks(get_callbacks(false));

    // Prepare builder.
    let mut builder = RepoBuilder::new();
    builder.fetch_options(fo);

    info!("pushing {} heavy files to git repo", paths.len());
    debug!("cloning in {}", dir.display());
    // Clone the project.
    let repo = builder
        .bare(true)
        .clone(&format!("git@gist.github.com:{}.git", gist_id), &dir)?;

    {
        debug!("find_branch");
        let branch = repo.find_branch("origin/master", BranchType::Remote)?;
        let commit = branch.get().peel_to_commit()?;
        let new_tree = {
            let tree = branch.get().peel_to_tree()?;
            let mut tree_builder = repo.treebuilder(Some(&tree))?;

            if need_temp_file {
                debug!("removing temp");
                tree_builder.remove("temp")?;
            }

            let odb = repo.odb()?;
            for path in paths {
                debug!("adding {:?} to odb", path);

                let file = File::open(&path)?;
                let mut odb_writer = odb.writer(file.metadata()?.len() as _, ObjectType::Blob)?;

                let mut reader = BufReader::new(file);
                io::copy(&mut reader, &mut odb_writer)?;

                let oid = odb_writer.finalize()?;
                let filename = path.file_name().unwrap();

                tree_builder.insert(filename, oid, 0o100644)?;
            }

            let new_tree_oid = tree_builder.write()?;
            debug!("tree built");

            repo.find_tree(new_tree_oid)?
        };

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
            ResetType::Soft,
            None,
        )?;
    }

    let mut remote = repo.find_remote("origin")?;

    // Prepare fetch options.
    let mut po = PushOptions::new();
    po.remote_callbacks(get_callbacks(true));

    debug!("pushing");
    remote.push(&["+refs/heads/master:refs/heads/master"], Some(&mut po))?;

    Ok(())
}
