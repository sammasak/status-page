use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ServiceConfig {
    name: String,
    url: String,         // display URL shown to users
    health_url: String,  // actual URL polled for health
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CheckResult {
    up: bool,
    checked_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct ServiceState {
    config: ServiceConfig,
    current_up: bool,
    last_checked: Option<DateTime<Utc>>,
    // Ring buffer: last 1440 checks (24h at 1-min intervals)
    history: VecDeque<bool>,
}

impl ServiceState {
    fn new(config: ServiceConfig) -> Self {
        Self {
            config,
            current_up: false,
            last_checked: None,
            history: VecDeque::with_capacity(1440),
        }
    }

    fn uptime_percent(&self) -> f64 {
        if self.history.is_empty() {
            return 100.0;
        }
        let up_count = self.history.iter().filter(|&&v| v).count();
        (up_count as f64 / self.history.len() as f64) * 100.0
    }

    fn record(&mut self, up: bool, at: DateTime<Utc>) {
        self.current_up = up;
        self.last_checked = Some(at);
        if self.history.len() >= 1440 {
            self.history.pop_front();
        }
        self.history.push_back(up);
    }
}

type AppState = Arc<RwLock<Vec<ServiceState>>>;

fn services() -> Vec<ServiceConfig> {
    vec![
        ServiceConfig {
            name: "Grafana".to_string(),
            url: "https://grafana.sammasak.dev".to_string(),
            health_url: "https://grafana.sammasak.dev/api/health".to_string(),
        },
        ServiceConfig {
            name: "Harbor (Registry)".to_string(),
            url: "https://registry.sammasak.dev".to_string(),
            health_url: "https://registry.sammasak.dev/api/v2.0/ping".to_string(),
        },
    ]
}

/// Check a service using its dedicated health endpoint.
/// Grafana: must contain `"database":"ok"` in JSON body.
/// Harbor ping: body must contain "Pong".
/// Fallback: any 2xx response counts as UP.
async fn check_service(client: &reqwest::Client, config: &ServiceConfig) -> bool {
    match client.get(&config.health_url).send().await {
        Ok(resp) => {
            if !resp.status().is_success() {
                return false;
            }
            let body = resp.text().await.unwrap_or_default();
            // Grafana: must contain "database":"ok" (with or without spaces)
            if config.health_url.contains("/api/health") {
                return body.contains("\"database\":\"ok\"")
                    || body.contains("\"database\": \"ok\"");
            }
            // Harbor ping: body is "Pong"
            if config.health_url.contains("/api/v2.0/ping") {
                return body.trim().trim_matches('"') == "Pong" || body.contains("Pong");
            }
            // Fallback: any 2xx
            true
        }
        Err(_) => false,
    }
}

async fn poll_loop(state: AppState) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .danger_accept_invalid_certs(false)
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .expect("Failed to build HTTP client");

    loop {
        {
            let mut services = state.write().await;
            for svc in services.iter_mut() {
                let up = check_service(&client, &svc.config).await;
                svc.record(up, Utc::now());
            }
        }
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

fn render_html(services: &[ServiceState], now: DateTime<Utc>) -> String {
    let all_up = services.iter().all(|s| s.current_up);

    let overall_class = if all_up { "overall-up" } else { "overall-down" };
    let overall_title = if all_up {
        "All Systems Operational"
    } else {
        "Degraded Performance"
    };
    let overall_desc = if all_up {
        "All services are running normally."
    } else {
        "One or more services are experiencing issues."
    };

    let mut rows = String::new();
    for (i, svc) in services.iter().enumerate() {
        let badge_class = if svc.current_up { "up" } else { "down" };
        let badge_text = if svc.current_up { "UP" } else { "DOWN" };
        let uptime = svc.uptime_percent();
        let last_checked = svc
            .last_checked
            .map(|t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| "Never".to_string());

        if i > 0 {
            rows.push_str("<tr>\n");
        } else {
            rows.push_str("<tr>\n");
        }
        rows.push_str(&format!(
            r#"<td class="service-name">{}</td>
<td><span class="badge {}">{}</span></td>
<td class="uptime-bar-cell">
<div class="uptime-bar-bg"><div class="uptime-bar-fill" style="width:{:.1}%"></div></div>
<span class="uptime-pct">{:.1}%</span>
</td>
<td class="last-checked">{}</td>
</tr>"#,
            svc.config.name,
            badge_class,
            badge_text,
            uptime,
            uptime,
            last_checked
        ));
    }

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<meta http-equiv="refresh" content="60">
<title>Homelab Status</title>
<style>
*, *::before, *::after {{ box-sizing: border-box; margin: 0; padding: 0; }}
body {{
font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
background: #0f1117;
color: #e2e8f0;
min-height: 100vh;
padding: 2rem 1rem;
}}
.container {{ max-width: 800px; margin: 0 auto; }}
header {{ text-align: center; margin-bottom: 2rem; }}
header h1 {{ font-size: 2rem; font-weight: 700; color: #f8fafc; margin-bottom: 0.25rem; }}
header p.subtitle {{ color: #94a3b8; font-size: 0.9rem; }}
.overall-status {{
border-radius: 12px;
padding: 1.25rem 1.5rem;
margin-bottom: 2rem;
display: flex;
align-items: center;
gap: 1rem;
}}
.overall-up {{ background: #052e16; border: 1px solid #166534; }}
.overall-down {{ background: #2d0a0a; border: 1px solid #7f1d1d; }}
.overall-dot {{ width: 14px; height: 14px; border-radius: 50%; flex-shrink: 0; }}
.overall-up .overall-dot {{ background: #22c55e; box-shadow: 0 0 8px #22c55e; }}
.overall-down .overall-dot {{ background: #ef4444; box-shadow: 0 0 8px #ef4444; }}
.overall-title {{ font-weight: 600; font-size: 1rem; }}
.overall-up .overall-title {{ color: #86efac; }}
.overall-down .overall-title {{ color: #fca5a5; }}
.overall-desc {{ font-size: 0.85rem; color: #94a3b8; }}
.card {{ background: #1e2130; border: 1px solid #2d3748; border-radius: 12px; overflow: hidden; margin-bottom: 2rem; }}
.card-header {{ padding: 1rem 1.5rem; border-bottom: 1px solid #2d3748; font-weight: 600; font-size: 0.9rem; color: #94a3b8; text-transform: uppercase; letter-spacing: 0.05em; }}
table {{ width: 100%; border-collapse: collapse; }}
th, td {{ padding: 0.85rem 1.5rem; text-align: left; }}
th {{ font-size: 0.75rem; text-transform: uppercase; letter-spacing: 0.05em; color: #64748b; border-bottom: 1px solid #2d3748; }}
tr:not(:last-child) td {{ border-bottom: 1px solid #1a2035; }}
.service-name {{ font-weight: 500; color: #e2e8f0; }}
.badge {{ display: inline-block; padding: 0.2rem 0.6rem; border-radius: 9999px; font-size: 0.75rem; font-weight: 700; letter-spacing: 0.05em; }}
.badge.up {{ background: #052e16; color: #4ade80; border: 1px solid #166534; }}
.badge.down {{ background: #2d0a0a; color: #f87171; border: 1px solid #7f1d1d; }}
.uptime-bar-cell {{ display: flex; align-items: center; gap: 0.75rem; }}
.uptime-bar-bg {{ flex: 1; height: 6px; background: #2d3748; border-radius: 3px; overflow: hidden; }}
.uptime-bar-fill {{ height: 100%; background: #22c55e; border-radius: 3px; }}
.uptime-pct {{ font-size: 0.85rem; color: #94a3b8; min-width: 45px; text-align: right; }}
.last-checked {{ font-size: 0.8rem; color: #64748b; }}
footer {{ text-align: center; color: #475569; font-size: 0.8rem; margin-top: 1rem; }}
</style>
</head>
<body>
<div class="container">
<header>
<h1>Homelab Status</h1>
<p class="subtitle">sammasak.dev infrastructure monitoring</p>
</header>
<div class="overall-status {overall_class}">
<div class="overall-dot"></div>
<div>
<div class="overall-title">{overall_title}</div>
<div class="overall-desc">{overall_desc}</div>
</div>
</div>
<div class="card">
<div class="card-header">Services</div>
<table>
<thead>
<tr>
<th>Service</th>
<th>Status</th>
<th>24h Uptime</th>
<th>Last Checked</th>
</tr>
</thead>
<tbody>
{rows}
</tbody>
</table>
</div>
<footer>
<p>Updated: {} &mdash; Auto-refreshes every 60s</p>
</footer>
</div>
</body>
</html>"#,
        now.format("%Y-%m-%d %H:%M:%S UTC")
    )
}

async fn handle_request(
    state: AppState,
    mut stream: tokio::net::TcpStream,
) {
    let mut buf = [0u8; 4096];
    let n = match stream.read(&mut buf).await {
        Ok(n) if n > 0 => n,
        _ => return,
    };
    let request = String::from_utf8_lossy(&buf[..n]);
    let first_line = request.lines().next().unwrap_or("");

    // Only serve GET /
    let (method, path) = {
        let mut parts = first_line.split_whitespace();
        let m = parts.next().unwrap_or("");
        let p = parts.next().unwrap_or("/");
        (m, p)
    };

    if method != "GET" || (path != "/" && path != "/index.html") {
        let _ = stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
            .await;
        return;
    }

    let now = Utc::now();
    let services = state.read().await;
    let html = render_html(&services, now);
    drop(services);

    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-cache\r\n\r\n{}",
        html.len(),
        html
    );
    let _ = stream.write_all(response.as_bytes()).await;
}

#[tokio::main]
async fn main() {
    let configs = services();
    let state: AppState = Arc::new(RwLock::new(
        configs.into_iter().map(ServiceState::new).collect(),
    ));

    // Start background polling
    {
        let state_clone = Arc::clone(&state);
        tokio::spawn(async move {
            // Do an initial check immediately before sleeping
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .danger_accept_invalid_certs(false)
                .redirect(reqwest::redirect::Policy::limited(5))
                .build()
                .expect("Failed to build HTTP client");
            {
                let mut services = state_clone.write().await;
                for svc in services.iter_mut() {
                    let up = check_service(&client, &svc.config).await;
                    svc.record(up, Utc::now());
                }
            }
            poll_loop(state_clone).await;
        });
    }

    let listener = TcpListener::bind("0.0.0.0:8080")
        .await
        .expect("Failed to bind to 0.0.0.0:8080");
    eprintln!("Listening on 0.0.0.0:8080");

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let state_clone = Arc::clone(&state);
                tokio::spawn(handle_request(state_clone, stream));
            }
            Err(e) => {
                eprintln!("Accept error: {}", e);
            }
        }
    }
}
