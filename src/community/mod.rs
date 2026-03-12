pub mod api;
pub mod display;

use crate::auth::token::load_token;

fn print_community_help() {
    println!(
        "\
Usage:
  taida community <posts|post|messages|message|author>

Examples:
  taida community posts --tag wasm
  taida community author shijimic"
    );
}

pub fn run_community(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: taida community <posts|post|messages|message|author>");
        std::process::exit(1);
    }

    match args[0].as_str() {
        "--help" | "-h" => print_community_help(),
        "posts" => run_posts(&args[1..]),
        "post" => run_post(&args[1..]),
        "messages" => run_messages(),
        "message" => run_message(&args[1..]),
        "author" => run_author(&args[1..]),
        other => {
            eprintln!("Unknown community command: {}", other);
            eprintln!("Usage: taida community <posts|post|messages|message|author>");
            std::process::exit(1);
        }
    }
}

/// GET /posts - 全投稿を一覧表示（認証不要）
/// --tag <tag> でタグ絞り込み、--by <author> で著者絞り込み
fn run_posts(args: &[String]) {
    let token = load_token();
    let token_str = token.as_ref().map(|t| t.github_token.as_str());

    let mut tag: Option<&str> = None;
    let mut author: Option<&str> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--tag" if i + 1 < args.len() => {
                tag = Some(&args[i + 1]);
                i += 2;
            }
            "--by" if i + 1 < args.len() => {
                author = Some(&args[i + 1]);
                i += 2;
            }
            _ => {
                eprintln!("Unknown flag: {}", args[i]);
                eprintln!("Usage: taida community posts [--tag <tag>] [--by <author>]");
                std::process::exit(1);
            }
        }
    }

    let mut path = "/posts".to_string();
    let mut params = Vec::new();
    if let Some(t) = tag {
        params.push(format!("tag={}", t));
    }
    if let Some(a) = author {
        params.push(format!("author={}", a));
    }
    if !params.is_empty() {
        path.push('?');
        path.push_str(&params.join("&"));
    }

    match api::api_get(&path, token_str) {
        Ok((status, body)) => {
            if (200..300).contains(&status) {
                display::display_posts(&body);
            } else {
                eprintln!("Error (HTTP {}): {}", status, body);
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

/// POST /posts - 投稿を作成（認証必須）
/// --tag <tag> でタグを追加（複数指定可）
fn run_post(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: taida community post \"content\" [--tag <tag>...]");
        std::process::exit(1);
    }

    let token = require_auth();

    let mut tags: Vec<String> = Vec::new();
    let mut content_parts: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--tag" && i + 1 < args.len() {
            tags.push(args[i + 1].clone());
            i += 2;
        } else {
            content_parts.push(&args[i]);
            i += 1;
        }
    }

    let content = content_parts.join(" ");
    if content.is_empty() {
        eprintln!("Usage: taida community post \"content\" [--tag <tag>...]");
        std::process::exit(1);
    }

    let body = if tags.is_empty() {
        serde_json::json!({ "content": content })
    } else {
        serde_json::json!({ "content": content, "tags": tags })
    };

    match api::api_post("/posts", &body, Some(&token.github_token)) {
        Ok((status, resp_body)) => {
            if (200..300).contains(&status) {
                display::display_post_created(&resp_body);
            } else {
                eprintln!("Error (HTTP {}): {}", status, resp_body);
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

/// GET /{self}/messages - 自分宛の公開メッセージを一覧表示（認証不要だが自分のユーザー名が必要）
fn run_messages() {
    let token = require_auth();
    let path = format!("/{}/messages", token.username);

    match api::api_get(&path, Some(&token.github_token)) {
        Ok((status, body)) => {
            if (200..300).contains(&status) {
                display::display_messages(&body);
            } else {
                eprintln!("Error (HTTP {}): {}", status, body);
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

/// POST /{user}/messages - 公開メッセージを送信（認証必須）
fn run_message(args: &[String]) {
    // パース: --to <user> <content...>
    if args.len() < 3 || args[0] != "--to" {
        eprintln!("Usage: taida community message --to <user> \"content\"");
        std::process::exit(1);
    }

    let token = require_auth();
    let to_user = &args[1];
    let content = args[2..].join(" ");

    let path = format!("/{}/messages", to_user);
    let body = serde_json::json!({ "content": content });

    match api::api_post(&path, &body, Some(&token.github_token)) {
        Ok((status, resp_body)) => {
            if (200..300).contains(&status) {
                display::display_message_sent(&resp_body);
            } else {
                eprintln!("Error (HTTP {}): {}", status, resp_body);
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

/// GET /{author} - 著者プロフィールを表示
fn run_author(args: &[String]) {
    let author_name = if args.is_empty() {
        // 名前の指定なし: 自分のプロフィールを表示
        let token = require_auth();
        token.username.clone()
    } else {
        args[0].clone()
    };

    let token = load_token();
    let token_str = token.as_ref().map(|t| t.github_token.as_str());
    let path = format!("/{}", author_name);

    match api::api_get(&path, token_str) {
        Ok((status, body)) => {
            if (200..300).contains(&status) {
                display::display_author(&body);
            } else {
                eprintln!("Error (HTTP {}): {}", status, body);
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

/// 認証を要求する。未ログインの場合はメッセージを表示して終了する。
fn require_auth() -> crate::auth::token::AuthToken {
    match load_token() {
        Some(token) => token,
        None => {
            eprintln!("Authentication required. Run `taida auth login` first.");
            std::process::exit(1);
        }
    }
}
