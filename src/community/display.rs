use serde_json::Value;

/// 投稿の一覧を人間が読みやすい形式で表示する。
/// サーバーは `{ "posts": [...] }` を返す。
pub fn display_posts(json: &str) {
    match serde_json::from_str::<Value>(json) {
        Ok(Value::Object(obj)) => {
            if let Some(Value::Array(posts)) = obj.get("posts") {
                if posts.is_empty() {
                    println!("No posts yet.");
                    return;
                }
                for post in posts {
                    print_post(post);
                }
            } else {
                println!("{}", json);
            }
        }
        Ok(_) => {
            println!("{}", json);
        }
        Err(_) => {
            println!("{}", json);
        }
    }
}

/// 投稿作成後の単一オブジェクトを表示する。
pub fn display_post_created(json: &str) {
    match serde_json::from_str::<Value>(json) {
        Ok(Value::Object(obj)) => {
            let author = obj
                .get("author")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let content = obj.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let tags = format_tags_from_map(&obj);
            if tags.is_empty() {
                println!("Posted as @{}: {}", author, content);
            } else {
                println!("Posted as @{}: {} {}", author, content, tags);
            }
        }
        Ok(_) => {
            println!("{}", json);
        }
        Err(_) => {
            println!("{}", json);
        }
    }
}

/// メッセージの一覧を人間が読みやすい形式で表示する。
/// サーバーは `{ "messages": [...] }` を返す。
pub fn display_messages(json: &str) {
    match serde_json::from_str::<Value>(json) {
        Ok(Value::Object(obj)) => {
            if let Some(Value::Array(messages)) = obj.get("messages") {
                if messages.is_empty() {
                    println!("No messages.");
                    return;
                }
                for msg in messages {
                    let from = msg
                        .get("from")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    let created = msg.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
                    println!("[{}] from @{}: {}", created, from, content);
                }
            } else {
                println!("{}", json);
            }
        }
        Ok(_) => {
            println!("{}", json);
        }
        Err(_) => {
            println!("{}", json);
        }
    }
}

/// メッセージ送信後の単一オブジェクトを表示する。
pub fn display_message_sent(json: &str) {
    match serde_json::from_str::<Value>(json) {
        Ok(Value::Object(obj)) => {
            let to = obj.get("to").and_then(|v| v.as_str()).unwrap_or("unknown");
            let content = obj.get("content").and_then(|v| v.as_str()).unwrap_or("");
            println!("Message sent to @{}: {}", to, content);
        }
        Ok(_) => {
            println!("{}", json);
        }
        Err(_) => {
            println!("{}", json);
        }
    }
}

/// 著者プロフィールを人間が読みやすい形式で表示する。
pub fn display_author(json: &str) {
    match serde_json::from_str::<Value>(json) {
        Ok(Value::Object(obj)) => {
            let name = obj
                .get("username")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            println!("@{}", name);
            if let Some(bio) = obj.get("bio").and_then(|v| v.as_str())
                && !bio.is_empty()
            {
                println!("  {}", bio);
            }
            if let Some(packages) = obj.get("packages").and_then(|v| v.as_array())
                && !packages.is_empty()
            {
                println!("  Packages:");
                for pkg in packages {
                    if let Some(name) = pkg.as_str() {
                        println!("    - {}", name);
                    } else if let Some(name) = pkg.get("name").and_then(|v| v.as_str()) {
                        println!("    - {}", name);
                    }
                }
            }
        }
        Ok(_) => {
            println!("{}", json);
        }
        Err(_) => {
            println!("{}", json);
        }
    }
}

fn print_post(post: &Value) {
    let author = post
        .get("author")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let content = post.get("content").and_then(|v| v.as_str()).unwrap_or("");
    let created = post
        .get("created_at")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let tags = format_tags(post);
    if tags.is_empty() {
        println!("[{}] @{}: {}", created, author, content);
    } else {
        println!("[{}] @{}: {} {}", created, author, content, tags);
    }
}

fn format_tags(obj: &Value) -> String {
    match obj.get("tags").and_then(|v| v.as_array()) {
        Some(tags) if !tags.is_empty() => {
            let tag_strs: Vec<&str> = tags.iter().filter_map(|t| t.as_str()).collect();
            format!("[{}]", tag_strs.join(", "))
        }
        _ => String::new(),
    }
}

fn format_tags_from_map(obj: &serde_json::Map<String, Value>) -> String {
    match obj.get("tags").and_then(|v| v.as_array()) {
        Some(tags) if !tags.is_empty() => {
            let tag_strs: Vec<&str> = tags.iter().filter_map(|t| t.as_str()).collect();
            format!("[{}]", tag_strs.join(", "))
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_posts_empty() {
        display_posts(r#"{"posts":[]}"#);
    }

    #[test]
    fn test_display_posts_wrapped() {
        let json = r#"{"posts":[{"author":"alice","content":"hello","tags":["parser"],"created_at":"2026-03-06"}]}"#;
        display_posts(json);
    }

    #[test]
    fn test_display_post_created() {
        let json =
            r#"{"author":"alice","content":"hello","tags":["mold"],"created_at":"2026-03-06"}"#;
        display_post_created(json);
    }

    #[test]
    fn test_display_messages_wrapped() {
        let json = r#"{"messages":[{"from":"bob","content":"hi","created_at":"2026-03-06"}]}"#;
        display_messages(json);
    }

    #[test]
    fn test_display_messages_empty() {
        display_messages(r#"{"messages":[]}"#);
    }

    #[test]
    fn test_display_message_sent() {
        let json = r#"{"to":"bob","content":"hi","created_at":"2026-03-06"}"#;
        display_message_sent(json);
    }

    #[test]
    fn test_display_author_object() {
        let json = r#"{"username":"alice","bio":"Taida dev","packages":["mylib"]}"#;
        display_author(json);
    }
}
