use crate::error::{Error, Result};

pub fn get_token() -> Result<String> {
    let out = std::process::Command::new("gh")
        .args(["auth", "token"])
        .output()
        .map_err(|e| {
            // gh binary not found or could not be executed
            let _ = e;
            Error::AuthFailure
        })?;

    if !out.status.success() || out.stdout.is_empty() {
        return Err(Error::AuthFailure);
    }

    let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if token.is_empty() {
        return Err(Error::AuthFailure);
    }

    Ok(token)
}
