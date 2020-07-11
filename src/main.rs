mod error;
mod gists_api;
mod git;
mod logger;

use crate::{error::*, git::handle_heavy_paths};
use clap::{crate_name, crate_version, App, Arg};
use futures::StreamExt;
use log::*;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    pin::Pin,
};
use tokio::{
    fs::File,
    io::{stdin, AsyncReadExt, BufReader},
    prelude::AsyncRead,
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
    let mut heavy_paths: Vec<PathBuf> = Vec::new();

    for (path, light_content) in successes.drain(..) {
        if let Some(light_content) = light_content {
            light_contents.push((path, light_content));
        } else {
            heavy_paths.push(path.to_path_buf());
        }
    }

    assert!(!light_contents.is_empty() || !heavy_paths.is_empty());

    let mut files = HashMap::new();

    let need_temp_file = light_contents.is_empty();
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
            assert!(files.insert(file_name, content).is_none());
        }
    }

    info!(
        "creating gist with {} light files and {} heavy files",
        if need_temp_file { 0 } else { files.len() },
        heavy_paths.len()
    );

    let gist_id = gists_api::create(files, false, None).await?;
    debug!("gist {} created", gist_id);

    if !heavy_paths.is_empty() {
        handle_heavy_paths(heavy_paths, gist_id.clone(), need_temp_file).await?;
    }

    info!("gist https://gist.github.com/{} created", gist_id);

    Ok(())
}

async fn get_light_content(path: &Path) -> Result<Option<String>> {
    let mut reader: Pin<Box<dyn AsyncRead>> = if path.to_str().unwrap() != "-" {
        let f = File::open(path).await?;
        if f.metadata().await?.len() > 10 * 1024 * 1024 {
            // Be aware that for files larger than ten megabytes, you'll need to clone the gist
            return Ok(None);
        }

        Box::pin(BufReader::new(f))
    } else {
        // windows ctrl-z, linux ctrl-d
        info!("reading text from stdin");
        Box::pin(BufReader::new(stdin()))
    };

    let mut content = String::new();
    if let Err(e) = reader.read_to_string(&mut content).await {
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
