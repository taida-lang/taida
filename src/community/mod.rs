pub mod api;
pub mod display;

use crate::auth::token::load_token;

fn is_help_flag(raw: &str) -> bool {
    matches!(raw, "--help" | "-h")
}

fn has_help_flag(args: &[String]) -> bool {
    args.iter().any(|arg| is_help_flag(arg.as_str()))
}

enum ParsedCommand<T> {
    Help,
    Run(T),
}

struct PostRequest {
    tags: Vec<String>,
    content: String,
}

struct MessageRequest {
    to_user: String,
    content: String,
}

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

fn print_posts_help() {
    println!(
        "\
Usage:
  taida community posts [--tag <tag>] [--by <author>]

Behavior:
  List public posts, optionally filtered by tag or author."
    );
}

fn print_post_help() {
    println!(
        "\
Usage:
  taida community post \"content\" [--tag <tag>...]

Behavior:
  Create a public post as the authenticated user.
  Use `--` before content to keep literal flags such as `--help`."
    );
}

fn print_messages_help() {
    println!(
        "\
Usage:
  taida community messages

Behavior:
  List public messages addressed to the authenticated user."
    );
}

fn print_message_help() {
    println!(
        "\
Usage:
  taida community message --to <user> \"content\"

Behavior:
  Send a public message to another user.
  Use `--` before content to keep literal flags such as `--help`."
    );
}

fn print_author_help() {
    println!(
        "\
Usage:
  taida community author [NAME]

Behavior:
  Show an author profile. When NAME is omitted, show the authenticated user."
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
        "messages" => run_messages(&args[1..]),
        "message" => run_message(&args[1..]),
        "author" => run_author(&args[1..]),
        other => {
            eprintln!("Unknown community command: {}", other);
            eprintln!("Usage: taida community <posts|post|messages|message|author>");
            std::process::exit(1);
        }
    }
}

fn parse_post_args(args: &[String]) -> Result<ParsedCommand<PostRequest>, &'static str> {
    if args.len() == 1 && is_help_flag(args[0].as_str()) {
        return Ok(ParsedCommand::Help);
    }
    if args.is_empty() {
        return Err("Usage: taida community post \"content\" [--tag <tag>...]");
    }

    let mut tags = Vec::new();
    let mut content_parts = Vec::new();
    let mut literal_mode = false;
    let mut i = 0;
    while i < args.len() {
        if literal_mode {
            content_parts.push(args[i].clone());
            i += 1;
            continue;
        }

        match args[i].as_str() {
            "--" => {
                literal_mode = true;
                i += 1;
            }
            flag if is_help_flag(flag) => return Ok(ParsedCommand::Help),
            "--tag" if i + 1 < args.len() => {
                tags.push(args[i + 1].clone());
                i += 2;
            }
            "--tag" => return Err("Usage: taida community post \"content\" [--tag <tag>...]"),
            _ => {
                content_parts.push(args[i].clone());
                i += 1;
            }
        }
    }

    if content_parts.is_empty() {
        return Err("Usage: taida community post \"content\" [--tag <tag>...]");
    }

    Ok(ParsedCommand::Run(PostRequest {
        tags,
        content: content_parts.join(" "),
    }))
}

fn parse_message_args(args: &[String]) -> Result<ParsedCommand<MessageRequest>, &'static str> {
    if args.len() == 1 && is_help_flag(args[0].as_str()) {
        return Ok(ParsedCommand::Help);
    }

    let mut to_user: Option<String> = None;
    let mut content_parts = Vec::new();
    let mut literal_mode = false;
    let mut i = 0;
    while i < args.len() {
        if literal_mode {
            content_parts.push(args[i].clone());
            i += 1;
            continue;
        }

        match args[i].as_str() {
            "--" => {
                literal_mode = true;
                i += 1;
            }
            flag if is_help_flag(flag) => return Ok(ParsedCommand::Help),
            "--to" if to_user.is_none() && i + 1 < args.len() => {
                to_user = Some(args[i + 1].clone());
                i += 2;
            }
            "--to" => return Err("Usage: taida community message --to <user> \"content\""),
            _ => {
                content_parts.push(args[i].clone());
                i += 1;
            }
        }
    }

    let Some(to_user) = to_user else {
        return Err("Usage: taida community message --to <user> \"content\"");
    };
    if content_parts.is_empty() {
        return Err("Usage: taida community message --to <user> \"content\"");
    }

    Ok(ParsedCommand::Run(MessageRequest {
        to_user,
        content: content_parts.join(" "),
    }))
}

/// GET /posts - 全投稿を一覧表示（認証不要）
/// --tag <tag> でタグ絞り込み、--by <author> で著者絞り込み
fn run_posts(args: &[String]) {
    if has_help_flag(args) {
        print_posts_help();
        return;
    }

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
    let request = match parse_post_args(args) {
        Ok(ParsedCommand::Help) => {
            print_post_help();
            return;
        }
        Ok(ParsedCommand::Run(request)) => request,
        Err(usage) => {
            eprintln!("{}", usage);
            std::process::exit(1);
        }
    };

    let token = require_auth();

    let body = if request.tags.is_empty() {
        serde_json::json!({ "content": request.content })
    } else {
        serde_json::json!({ "content": request.content, "tags": request.tags })
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
fn run_messages(args: &[String]) {
    match args {
        [] => {}
        _ if has_help_flag(args) => {
            print_messages_help();
            return;
        }
        _ => {
            eprintln!("Usage: taida community messages");
            std::process::exit(1);
        }
    }

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
    let request = match parse_message_args(args) {
        Ok(ParsedCommand::Help) => {
            print_message_help();
            return;
        }
        Ok(ParsedCommand::Run(request)) => request,
        Err(usage) => {
            eprintln!("{}", usage);
            std::process::exit(1);
        }
    };

    let token = require_auth();
    let path = format!("/{}/messages", request.to_user);
    let body = serde_json::json!({ "content": request.content });

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
    if has_help_flag(args) {
        print_author_help();
        return;
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| part.to_string()).collect()
    }

    #[test]
    fn test_parse_post_args_help_flag_requests_help() {
        let args = strings(&["hello", "--help"]);
        assert!(matches!(parse_post_args(&args), Ok(ParsedCommand::Help)));
    }

    #[test]
    fn test_parse_post_args_double_dash_keeps_literal_help_token() {
        let args = strings(&["--", "--help"]);
        let Ok(ParsedCommand::Run(request)) = parse_post_args(&args) else {
            panic!("expected parsed post request");
        };
        assert_eq!(request.content, "--help");
        assert!(request.tags.is_empty());
    }

    #[test]
    fn test_parse_message_args_help_flag_requests_help() {
        let args = strings(&["--to", "alice", "hi", "--help"]);
        assert!(matches!(parse_message_args(&args), Ok(ParsedCommand::Help)));
    }

    #[test]
    fn test_parse_message_args_double_dash_keeps_literal_help_token() {
        let args = strings(&["--to", "alice", "--", "use", "--help", "now"]);
        let Ok(ParsedCommand::Run(request)) = parse_message_args(&args) else {
            panic!("expected parsed message request");
        };
        assert_eq!(request.to_user, "alice");
        assert_eq!(request.content, "use --help now");
    }
}
