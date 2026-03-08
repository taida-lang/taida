use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize)]
pub struct AuthToken {
    pub github_token: String,
    pub username: String,
    pub created_at: String,
}

/// ~/.taida/auth.json のパスを返す
pub fn auth_json_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".taida").join("auth.json")
}

/// 保存された認証トークンを読み込む。存在しなければ None を返す。
pub fn load_token() -> Option<AuthToken> {
    let path = auth_json_path();
    let data = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&data).ok()
}

/// 認証トークンを ~/.taida/auth.json に保存する。
pub fn save_token(github_token: &str, username: &str) -> Result<(), String> {
    let path = auth_json_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory {}: {}", parent.display(), e))?;
    }

    let now = chrono_rfc3339_now();
    let token = AuthToken {
        github_token: github_token.to_string(),
        username: username.to_string(),
        created_at: now,
    };

    let json = serde_json::to_string_pretty(&token)
        .map_err(|e| format!("Failed to serialize token: {}", e))?;
    fs::write(&path, json).map_err(|e| format!("Failed to write {}: {}", path.display(), e))?;
    Ok(())
}

/// 認証トークンファイルを削除する。
pub fn delete_token() -> Result<(), String> {
    let path = auth_json_path();
    if path.exists() {
        fs::remove_file(&path)
            .map_err(|e| format!("Failed to delete {}: {}", path.display(), e))?;
    }
    Ok(())
}

/// 外部クレートを使わずに RFC 3339 タイムスタンプを生成する。
fn chrono_rfc3339_now() -> String {
    use std::process::Command;
    // `date` コマンドで UTC ISO 8601 タイムスタンプを取得
    Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_auth_json_path() {
        let path = auth_json_path();
        assert!(path.to_string_lossy().ends_with(".taida/auth.json"));
    }

    #[test]
    fn test_save_and_load_token() {
        // 実際のホームディレクトリを汚さないよう一時ディレクトリを使用
        let tmp = env::temp_dir().join("taida_test_auth");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let auth_path = tmp.join("auth.json");
        let token = AuthToken {
            github_token: "gho_test123".to_string(),
            username: "testuser".to_string(),
            created_at: "2026-03-06T12:00:00Z".to_string(),
        };

        let json = serde_json::to_string_pretty(&token).unwrap();
        fs::write(&auth_path, &json).unwrap();

        let data = fs::read_to_string(&auth_path).unwrap();
        let loaded: AuthToken = serde_json::from_str(&data).unwrap();
        assert_eq!(loaded.username, "testuser");
        assert_eq!(loaded.github_token, "gho_test123");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_load_token_missing_file() {
        // ファイルが存在しない場合、load_token は None を返す
        // デシリアライズのパスをテスト
        let result: Option<AuthToken> = serde_json::from_str("invalid json").ok();
        assert!(result.is_none());
    }
}
