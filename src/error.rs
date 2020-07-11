use crate::github_gists::GitHubError;
use error_chain::error_chain;
pub use error_chain::{bail, ensure};

error_chain! {
    foreign_links {
        Fmt(::std::fmt::Error);
        Io(::std::io::Error);
        ParseFloatError(::std::num::ParseFloatError);
        ParseIntError(::std::num::ParseIntError);
        ParseBoolError(::std::str::ParseBoolError);
        Tokio(tokio::task::JoinError);
        Clap(clap::Error);
        Reqwest(reqwest::Error);
        ReqwestHeader(reqwest::header::InvalidHeaderValue);
        SerdeJson(serde_json::Error);
        Git2(git2::Error);
    }

    errors {
        GitHub(error: GitHubError) {
            description("GitHub api error")
            display("GitHub api error: {}", error.message)
        }
    }
}
