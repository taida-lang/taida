pub mod device_flow;
pub mod token;

use token::{delete_token, load_token, save_token};

fn is_help_flag(raw: &str) -> bool {
    matches!(raw, "--help" | "-h")
}

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

fn print_auth_login_help() {
    println!(
        "\
Usage:
  taida auth login

Behavior:
  Start the GitHub Device Authorization Flow and store the resulting token locally."
    );
}

fn print_auth_logout_help() {
    println!(
        "\
Usage:
  taida auth logout

Behavior:
  Delete the locally stored authentication token, if present."
    );
}

fn print_auth_status_help() {
    println!(
        "\
Usage:
  taida auth status

Behavior:
  Print the currently authenticated user and token creation timestamp, if logged in."
    );
}

pub fn run_auth(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: taida auth <login|logout|status>");
        std::process::exit(1);
    }

    match args[0].as_str() {
        "--help" | "-h" => print_auth_help(),
        "login" => run_login(&args[1..]),
        "logout" => run_logout(&args[1..]),
        "status" => run_status(&args[1..]),
        other => {
            eprintln!("Unknown auth command: {}", other);
            eprintln!("Usage: taida auth <login|logout|status>");
            std::process::exit(1);
        }
    }
}

fn run_login(args: &[String]) {
    match args {
        [] => {}
        [arg] if is_help_flag(arg.as_str()) => {
            print_auth_login_help();
            return;
        }
        _ => {
            eprintln!("Usage: taida auth login");
            std::process::exit(1);
        }
    }

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

fn run_logout(args: &[String]) {
    match args {
        [] => {}
        [arg] if is_help_flag(arg.as_str()) => {
            print_auth_logout_help();
            return;
        }
        _ => {
            eprintln!("Usage: taida auth logout");
            std::process::exit(1);
        }
    }

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

fn run_status(args: &[String]) {
    match args {
        [] => {}
        [arg] if is_help_flag(arg.as_str()) => {
            print_auth_status_help();
            return;
        }
        _ => {
            eprintln!("Usage: taida auth status");
            std::process::exit(1);
        }
    }

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
