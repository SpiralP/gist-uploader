// creates gist from json rest api then clones the git url into memory,
// then makes a commit and pushes git repo

mod error;
mod github_gists;

use crate::error::*;

#[tokio::main]
async fn main() -> Result<()> {
    println!("Hello, world!");

    Ok(())
}
