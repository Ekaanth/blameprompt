use reqwest::blocking::Client;
use serde::Deserialize;
use std::thread;
use std::time::Duration;

use crate::core::auth::{self, Credentials};

const DEFAULT_API_URL: &str = "https://blameprompt.com";

#[derive(Deserialize)]
struct DeviceFlowInit {
    #[serde(rename = "deviceCode")]
    device_code: String,
    #[serde(rename = "userCode")]
    user_code: String,
    #[serde(rename = "verificationUri")]
    verification_uri: String,
    interval: u64,
}

#[derive(Deserialize)]
struct DevicePollResponse {
    status: String,
    #[serde(rename = "apiKey")]
    api_key: Option<String>,
    username: Option<String>,
}

pub fn run(token: Option<&str>, api_url: Option<&str>) {
    let base_url = api_url.unwrap_or(DEFAULT_API_URL);

    // If --token is provided, save directly (headless / CI)
    if let Some(key) = token {
        // Validate the token by calling /api/me
        let client = Client::new();
        let res = client
            .get(format!("{}/api/me", base_url))
            .header("X-API-Key", key)
            .send();

        match res {
            Ok(r) if r.status().is_success() => {
                #[derive(Deserialize)]
                struct MeResponse {
                    username: String,
                }
                match r.json::<MeResponse>() {
                    Ok(me) => {
                        let creds = Credentials {
                            api_key: key.to_string(),
                            username: me.username.clone(),
                            api_url: base_url.to_string(),
                        };
                        if let Err(e) = auth::save(&creds) {
                            eprintln!("  \x1b[1;31mError:\x1b[0m {}", e);
                            std::process::exit(1);
                        }
                        println!();
                        println!(
                            "  \x1b[1;32m\u{2713}\x1b[0m Logged in as \x1b[1m@{}\x1b[0m",
                            me.username
                        );
                        println!();
                    }
                    Err(e) => {
                        eprintln!("  \x1b[1;31mError:\x1b[0m Failed to parse response: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            Ok(r) => {
                eprintln!(
                    "  \x1b[1;31mError:\x1b[0m Invalid token (HTTP {})",
                    r.status()
                );
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("  \x1b[1;31mError:\x1b[0m Could not reach API: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }

    // GitHub Device Flow
    println!();
    println!("  \x1b[1mBlamePrompt Login\x1b[0m");
    println!("  \x1b[2m────────────────────\x1b[0m");
    println!();

    let client = Client::new();

    // Step 1: Initiate device flow
    let init_res = client
        .post(format!("{}/api/auth/github/device", base_url))
        .header("Content-Type", "application/json")
        .body("{}")
        .send();

    let init: DeviceFlowInit = match init_res {
        Ok(r) if r.status().is_success() => match r.json() {
            Ok(v) => v,
            Err(e) => {
                eprintln!(
                    "  \x1b[1;31mError:\x1b[0m Failed to parse device flow response: {}",
                    e
                );
                std::process::exit(1);
            }
        },
        Ok(_) | Err(_) => {
            // Device flow API not available — fall back to browser login
            let sign_in_url = format!("{}/login", base_url);
            println!(
                "  Opening \x1b[36m{}\x1b[0m in your browser...",
                sign_in_url
            );
            println!();
            if open::that(&sign_in_url).is_err() {
                println!("  \x1b[2mCould not open browser. Visit manually:\x1b[0m");
                println!("  \x1b[36m{}\x1b[0m", sign_in_url);
            }
            println!(
                "  After signing in, copy your API token from your profile and run:"
            );
            println!();
            println!(
                "    \x1b[36mblameprompt login --token <your-token>\x1b[0m"
            );
            println!();
            return;
        }
    };

    // Step 2: Show code and open browser
    println!(
        "  Go to \x1b[36m{}\x1b[0m and enter code:",
        init.verification_uri
    );
    println!();
    println!("    \x1b[1;33m{}\x1b[0m", init.user_code);
    println!();

    // Try to open browser
    if open::that(&init.verification_uri).is_err() {
        println!("  \x1b[2m(Could not open browser automatically)\x1b[0m");
    }

    println!("  \x1b[2mWaiting for authorization...\x1b[0m");

    // Step 3: Poll for completion
    let interval = Duration::from_secs(init.interval.max(5));
    loop {
        thread::sleep(interval);

        let poll_res = client
            .post(format!("{}/api/auth/github/device/poll", base_url))
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({ "deviceCode": init.device_code }))
            .send();

        match poll_res {
            Ok(r) if r.status().is_success() => {
                match r.json::<DevicePollResponse>() {
                    Ok(poll) if poll.status == "complete" => {
                        let api_key = poll.api_key.unwrap_or_default();
                        let username = poll.username.unwrap_or_default();

                        let creds = Credentials {
                            api_key,
                            username: username.clone(),
                            api_url: base_url.to_string(),
                        };

                        if let Err(e) = auth::save(&creds) {
                            eprintln!("  \x1b[1;31mError:\x1b[0m {}", e);
                            std::process::exit(1);
                        }

                        println!();
                        println!(
                            "  \x1b[1;32m\u{2713}\x1b[0m Logged in as \x1b[1m@{}\x1b[0m",
                            username
                        );
                        println!();
                        return;
                    }
                    Ok(_) => {
                        // Still pending, continue polling
                    }
                    Err(e) => {
                        eprintln!("  \x1b[1;31mError:\x1b[0m Poll parse error: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            Ok(r) => {
                eprintln!(
                    "  \x1b[1;31mError:\x1b[0m Poll failed (HTTP {})",
                    r.status()
                );
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("  \x1b[1;31mError:\x1b[0m Poll request failed: {}", e);
                std::process::exit(1);
            }
        }
    }
}

pub fn logout() {
    match auth::clear() {
        Ok(()) => {
            println!();
            println!("  \x1b[1;32m\u{2713}\x1b[0m Logged out successfully.");
            println!();
        }
        Err(e) => {
            eprintln!("  \x1b[1;31mError:\x1b[0m {}", e);
            std::process::exit(1);
        }
    }
}
