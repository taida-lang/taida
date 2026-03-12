pub mod device_flow;
pub mod token;

use token::{delete_token, load_token, save_token};

fn print_auth_help() {
    println!(
        "\
Usage:
  taida auth <login|logout|status>

Examples:
  taida auth login
  taida auth status"
    );
}

pub fn run_auth(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: taida auth <login|logout|status>");
        std::process::exit(1);
    }

    match args[0].as_str() {
        "--help" | "-h" => print_auth_help(),
        "login" => run_login(),
        "logout" => run_logout(),
        "status" => run_status(),
        other => {
            eprintln!("Unknown auth command: {}", other);
            eprintln!("Usage: taida auth <login|logout|status>");
            std::process::exit(1);
        }
    }
}

fn run_login() {
    // すでにログイン済みか確認
    if let Some(existing) = load_token() {
        println!(
            "Already logged in as {}. Run `taida auth logout` first to re-authenticate.",
            existing.username
        );
        return;
    }

    println!("Starting GitHub Device Authorization Flow...\n");

    // ステップ1: デバイスコードをリクエスト
    let flow = match device_flow::start_device_flow() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };

    // ステップ2: ユーザーへの案内を表示
    println!("Open this URL in your browser:");
    println!("  {}", flow.verification_uri);
    println!();
    println!("Enter code: {}", flow.user_code);
    println!();
    println!("Waiting for authorization...");

    // ステップ3: トークンをポーリングで取得
    let access_token = match device_flow::poll_for_token(&flow.device_code, flow.interval) {
        Ok(t) => {
            println!(); // ドットの後の改行
            t
        }
        Err(e) => {
            println!();
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };

    // ステップ4: ユーザー名を取得
    let username = match device_flow::get_github_username(&access_token) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };

    // ステップ5: トークンを保存
    match save_token(&access_token, &username) {
        Ok(()) => {
            println!("Logged in as {}.", username);
        }
        Err(e) => {
            eprintln!("Error saving token: {}", e);
            std::process::exit(1);
        }
    }
}

fn run_logout() {
    match load_token() {
        Some(token) => match delete_token() {
            Ok(()) => println!("Logged out (was {}).", token.username),
            Err(e) => {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        },
        None => {
            println!("Not logged in.");
        }
    }
}

fn run_status() {
    match load_token() {
        Some(token) => {
            println!("Logged in as {}.", token.username);
            println!("Token created: {}", token.created_at);
        }
        None => {
            println!("Not logged in. Run `taida auth login` to authenticate.");
        }
    }
}
