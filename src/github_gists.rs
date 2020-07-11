use crate::error::*;
use dirs::home_dir;
use reqwest::{header, header::HeaderValue, Client};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, env, fs::File, io::prelude::*};

const APP_USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

fn get_token() -> Result<String> {
    if let Ok(var) = env::var("GITHUB_GIST_TOKEN") {
        return Ok(var);
    }

    if let Ok(var) = env::var("GITHUB_TOKEN") {
        return Ok(var);
    }

    if let Ok(mut file) = File::open(home_dir().chain_err(|| "home_dir()")?.join(".gist")) {
        let mut token = String::new();
        file.read_to_string(&mut token)?;
        return Ok(token);
    }

    bail!("GITHUB_GIST_TOKEN, GITHUB_TOKEN, or ~/.gist don't exist");
}

fn make_client() -> Result<Client> {
    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::ACCEPT,
        HeaderValue::from_static("application/vnd.github.v3+json"),
    );

    let mut h = HeaderValue::from_str(&format!("token {}", get_token()?))?;
    h.set_sensitive(true);
    headers.insert(header::AUTHORIZATION, h);

    Ok(Client::builder()
        .user_agent(APP_USER_AGENT)
        .default_headers(headers)
        .build()?)
}

/// returns gist id
pub async fn create(
    mut files: HashMap<String, String>,
    public: bool,
    description: Option<String>,
) -> Result<String> {
    ensure!(!files.is_empty(), "files can't be empty");

    let mut gist_files = HashMap::new();
    for (name, content) in files.drain() {
        ensure!(!content.is_empty(), "content can't be empty");
        gist_files.insert(name, GistFile { content });
    }

    // content must not be blank
    // must have at least 1 file

    let response = make_client()?
        .post("https://api.github.com/gists")
        .json(&PostGistsRequest {
            files: gist_files,
            description,
            public,
        })
        .send()
        .await?;

    let bytes = response.bytes().await?;

    let err: serde_json::Result<GitHubError> = serde_json::from_slice(&bytes);
    let response: PostGistsResponse = match err {
        Ok(err) => {
            return Err(ErrorKind::GitHub(err).into());
        }

        Err(_) => serde_json::from_slice(&bytes)?,
    };

    Ok(response.id)
}

#[allow(dead_code)]
pub async fn delete(id: &str) -> Result<()> {
    let response = make_client()?
        .delete(&format!("https://api.github.com/gists/{}", id))
        .send()
        .await?;

    let status = response.status();
    if status.is_success() {
        Ok(())
    } else {
        let bytes = response.bytes().await?;

        let err: serde_json::Result<GitHubError> = serde_json::from_slice(&bytes);
        match err {
            Ok(err) => Err(ErrorKind::GitHub(err).into()),
            Err(_) => bail!("non-success status {:?}", status),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PostGistsRequest {
    pub files: HashMap<String, GistFile>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    pub public: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GistFile {
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PostGistsResponse {
    pub id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeleteGistsRequest {
    pub gist_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GitHubError {
    pub message: String,
    pub errors: Option<Vec<HashMap<String, String>>>,
    pub documentation_url: Option<String>,
}

#[tokio::test]
async fn test_create_empty() {
    let mut files = HashMap::new();

    let mut content = String::new();
    {
        let mut f = File::open(r"C:\Users\SpiralP\Desktop\cc\cat.png").unwrap();
        if let Err(e) = f.read_to_string(&mut content) {
            // let ag: () = e;
            panic!("{}", e);
        }
    }

    files.insert("file.txt".to_string(), content);
    let id = create(files, false, None).await.unwrap();
    println!("{:#?}", id);
    delete(&id).await.unwrap();
}
