// creates gist from json rest api then clones the git url into memory,
// then makes a commit and pushes git repo

mod error;
mod github_gists;
mod logger;

use crate::error::*;
use clap::{crate_name, crate_version, App, Arg};
use dirs::home_dir;
use futures::StreamExt;
use git2::{Cred, RemoteCallbacks};
use log::*;
use std::{collections::HashMap, io::Write, path::Path};
use tokio::{
    fs::File,
    io::{stdin, AsyncReadExt, BufReader},
};

#[tokio::main]
async fn main() -> Result<()> {
    let app = App::new(crate_name!())
        .version(crate_version!())
        .arg(
            Arg::with_name("verbose")
                .long("verbose")
                .short("v")
                .help("Show debug messages, use multiple times for higher verbosity")
                .multiple(true),
        )
        .arg(Arg::with_name("file").multiple(true).default_value("-"));

    let matches = app.get_matches();

    let verbose = matches.is_present("verbose");
    logger::initialize(
        cfg!(debug_assertions) || verbose,
        matches.occurrences_of("verbose") > 1,
    );

    let mut results: Vec<Result<_>> =
        futures::stream::iter(matches.values_of("file").unwrap().map(|file| async move {
            let path = Path::new(file);
            // TODO break early if errors
            Ok((path, get_light_content(&path).await?))
        }))
        .buffer_unordered(32)
        .collect()
        .await;
    let mut successes = results.drain(..).collect::<Result<Vec<_>>>()?;

    let mut light_contents: Vec<(&Path, String)> = Vec::new();
    let mut heavy_paths: Vec<&Path> = Vec::new();

    for (path, light_content) in successes.drain(..) {
        if let Some(light_content) = light_content {
            light_contents.push((path, light_content));
        } else {
            heavy_paths.push(path);
        }
    }

    assert!(!light_contents.is_empty() || !heavy_paths.is_empty());

    let mut files = HashMap::new();

    let mut need_temp_file = light_contents.is_empty();
    if need_temp_file {
        files.insert("temp".to_string(), "temp".to_string());
    } else {
        for (path, content) in light_contents.drain(..) {
            // TODO check if we can do dirs?
            let file_name = if path.to_str().unwrap() == "-" {
                "stdin.txt".to_string()
            } else {
                path.file_name().unwrap().to_str().unwrap().to_string()
            };
            assert!(files.insert(file_name, content,).is_none());
        }
    }
    let len = files.len();
    let gist_id = github_gists::create(files, false, None).await?;
    info!(
        "gist https://gist.github.com/{} created with {} light files",
        gist_id, len
    );

    // github_gists::create(files,

    handle_heavy_paths(&heavy_paths, &gist_id, need_temp_file).await?;

    Ok(())
}

async fn get_light_content(path: &Path) -> Result<Option<String>> {
    if path.to_str().unwrap() != "-" {
        let f = File::open(path).await?;
        if f.metadata().await?.len() > 10 * 1024 * 1024 {
            // Be aware that for files larger than ten megabytes, you'll need to clone the gist
            Ok(None)
        } else {
            let mut content = String::new();
            let mut reader = BufReader::new(f);
            if let Err(e) = reader.read_to_string(&mut content).await {
                if e.kind() == std::io::ErrorKind::InvalidData {
                    Ok(None)
                } else {
                    Err(e.into())
                }
            } else {
                Ok(Some(content))
            }
        }
    } else {
        let mut f = stdin();
        let mut content = String::new();
        if let Err(e) = f.read_to_string(&mut content).await {
            if e.kind() == std::io::ErrorKind::InvalidData {
                Ok(None)
            } else {
                Err(e.into())
            }
        } else if content.len() > 10 * 1024 * 1024 {
            // Be aware that for files larger than ten megabytes, you'll need to clone the gist
            Ok(None)
        } else {
            Ok(Some(content))
        }
    }
}

async fn handle_heavy_paths(paths: &[&Path], gist_id: &str, need_temp_file: bool) -> Result<()> {
    let tmp_dir = tempfile::Builder::new()
        .prefix(concat!(crate_name!(), "-"))
        .tempdir()?;

    let result = doot(paths, gist_id, tmp_dir.path(), need_temp_file).await;

    std::mem::forget(tmp_dir);
    // debug!("tmp_dir.close()");
    // if let Err(e) = tmp_dir.close() {
    //     error!("couldn't delete tempdir: {}", e);
    // }

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

async fn doot(paths: &[&Path], gist_id: &str, dir: &Path, need_temp_file: bool) -> Result<()> {
    // Prepare fetch options.
    let mut fo = git2::FetchOptions::new();
    fo.remote_callbacks(get_callbacks());

    // Prepare builder.
    let mut builder = git2::build::RepoBuilder::new();
    builder.fetch_options(fo);

    info!("cloning git repo");
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
    remote.push(&["+refs/heads/master:refs/heads/master"], Some(&mut po))?;

    Ok(())
}
