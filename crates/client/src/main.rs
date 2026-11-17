use std::sync::Arc;

use clap::{Parser, Subcommand};
use pulsar_client::core::adapters::{NativeReqwestTransport, ServerModeProvider};
use pulsar_client::core::backend::SandboxProvider;
use pulsar_client::core::state::AppState;
use pulsar_client::tui::run_live_tui;
use pulsar_client::ui::shell::render_app_shell;
use pulsar_protocol::InteractionMode;
use pulsar_styles::prelude::{SurfaceMetrics, TargetPlatform};

#[derive(Debug, Parser)]
#[command(
    name = "pulsar-client",
    version,
    about = "Native terminal, web, and automation client for pulsar-server"
)]
struct Cli {
    /// Base URL of the pulsar-server instance.
    #[arg(long, default_value = "http://127.0.0.1:8080")]
    server_url: String,
    /// Optional bearer token or API key for authenticated server calls.
    #[arg(long)]
    token: Option<String>,
    /// Optional account email for login on startup.
    #[arg(long)]
    email: Option<String>,
    /// Optional account password for login on startup.
    #[arg(long)]
    password: Option<String>,
    /// Optional file path to write structured client diagnostic logs.
    #[arg(long)]
    log_file: Option<std::path::PathBuf>,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Launch the interactive terminal user interface wired to pulsar-server.
    Tui,
    /// Print the reactive HTML shell configured with the target server URL.
    Ui,
    /// Print client profile status and effective capabilities.
    Status,
    /// Authenticate against pulsar-server and print the issued token pair JSON.
    Login {
        #[arg(long)]
        email: String,
        #[arg(long)]
        password: String,
    },
    /// List connected boxer nodes via pulsar-server.
    Nodes,
    /// Submit a prompt to an existing or newly created session.
    Prompt {
        #[arg(long)]
        session_id: Option<String>,
        prompt: String,
    },
    /// List configured workflows via pulsar-server.
    Workflows,
    /// List autocron schedules via pulsar-server.
    Autocron,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    if let Some(log_path) = &cli.log_file {
        if let Some(parent) = log_path.parent()
            && !parent.as_os_str().is_empty()
        {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)?;
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_writer(std::sync::Mutex::new(file))
            .try_init();
        tracing::info!(path = %log_path.display(), "pulsar-client file logger initialized");
    }
    let server_url = if cli.server_url == "http://127.0.0.1:8080" {
        std::env::var("PULSAR_SERVER_URL").unwrap_or(cli.server_url)
    } else {
        cli.server_url
    };
    let token = cli.token.or_else(|| std::env::var("PULSAR_TOKEN").ok());
    let email = cli.email.or_else(|| std::env::var("PULSAR_EMAIL").ok());
    let password = cli
        .password
        .or_else(|| std::env::var("PULSAR_PASSWORD").ok());

    let mut state = AppState::default();
    state.login.server_url = server_url.clone();

    let transport = Arc::new(NativeReqwestTransport::default());
    let provider = ServerModeProvider::new(server_url.clone(), transport);

    match cli.command.unwrap_or(Commands::Status) {
        Commands::Tui => {
            run_live_tui(state, server_url, email, password, token).await?;
        }
        Commands::Ui => {
            let metrics = SurfaceMetrics::desktop(TargetPlatform::Web);
            println!(
                "{}",
                render_app_shell(&state, metrics).render_html_fragment()
            );
        }
        Commands::Status => {
            println!(
                "pulsar-client ready: {}",
                serde_json::to_string_pretty(&state.effective_profile)?
            );
        }
        Commands::Login { email, password } => {
            let pair = provider.login(&email, &password).await?;
            println!("{}", serde_json::to_string_pretty(&pair)?);
        }
        Commands::Nodes => {
            let tok = token.ok_or("missing --token (or PULSAR_TOKEN) for nodes query")?;
            let nodes = provider.list_nodes(&tok).await?;
            println!("{}", serde_json::to_string_pretty(&nodes)?);
        }
        Commands::Prompt { session_id, prompt } => {
            let tok = token.ok_or("missing --token (or PULSAR_TOKEN) for prompt submission")?;
            let sid = match session_id {
                Some(id) => id,
                None => {
                    let created = provider.create_session(&tok, "CLI Session", "act").await?;
                    created["id"]
                        .as_str()
                        .or_else(|| created["session"]["id"].as_str())
                        .ok_or("missing session id in server response")?
                        .to_owned()
                }
            };
            let sub = provider
                .submit_task(&tok, &sid, &prompt, InteractionMode::Act)
                .await?;
            println!("{}", serde_json::to_string_pretty(&sub)?);
        }
        Commands::Workflows => {
            let tok = token.ok_or("missing --token (or PULSAR_TOKEN) for workflows query")?;
            let workflows = provider.list_workflows(&tok).await?;
            println!("{}", serde_json::to_string_pretty(&workflows)?);
        }
        Commands::Autocron => {
            let tok = token.ok_or("missing --token (or PULSAR_TOKEN) for autocron query")?;
            let schedules = provider.list_autocron_schedules(&tok).await?;
            println!("{}", serde_json::to_string_pretty(&schedules)?);
        }
    }

    Ok(())
}
