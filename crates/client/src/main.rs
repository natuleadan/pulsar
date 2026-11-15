use std::sync::Arc;

use agents_client::core::adapters::{NativeReqwestTransport, ServerModeProvider};
use agents_client::core::backend::SandboxProvider;
use agents_client::core::state::AppState;
use agents_client::tui::run_live_tui;
use agents_client::ui::shell::render_app_shell;
use agents_protocol::InteractionMode;
use agents_styles::prelude::{SurfaceMetrics, TargetPlatform};
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "agents-client",
    version,
    about = "Native terminal, web, and automation client for agents-server"
)]
struct Cli {
    /// Base URL of the agents-server instance.
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
    /// Launch the interactive terminal user interface wired to agents-server.
    Tui,
    /// Print the reactive HTML shell configured with the target server URL.
    Ui,
    /// Print client profile status and effective capabilities.
    Status,
    /// Authenticate against agents-server and print the issued token pair JSON.
    Login {
        #[arg(long)]
        email: String,
        #[arg(long)]
        password: String,
    },
    /// List connected boxer nodes via agents-server.
    Nodes,
    /// Submit a prompt to an existing or newly created session.
    Prompt {
        #[arg(long)]
        session_id: Option<String>,
        prompt: String,
    },
    /// List configured workflows via agents-server.
    Workflows,
    /// List autocron schedules via agents-server.
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
        tracing::info!(path = %log_path.display(), "agents-client file logger initialized");
    }
    let server_url = if cli.server_url == "http://127.0.0.1:8080" {
        std::env::var("AGENTS_SERVER_URL").unwrap_or(cli.server_url)
    } else {
        cli.server_url
    };
    let token = cli.token.or_else(|| std::env::var("AGENTS_TOKEN").ok());
    let email = cli.email.or_else(|| std::env::var("AGENTS_EMAIL").ok());
    let password = cli
        .password
        .or_else(|| std::env::var("AGENTS_PASSWORD").ok());

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
                "agents-client ready: {}",
                serde_json::to_string_pretty(&state.effective_profile)?
            );
        }
        Commands::Login { email, password } => {
            let pair = provider.login(&email, &password).await?;
            println!("{}", serde_json::to_string_pretty(&pair)?);
        }
        Commands::Nodes => {
            let tok = token.ok_or("missing --token (or AGENTS_TOKEN) for nodes query")?;
            let nodes = provider.list_nodes(&tok).await?;
            println!("{}", serde_json::to_string_pretty(&nodes)?);
        }
        Commands::Prompt { session_id, prompt } => {
            let tok = token.ok_or("missing --token (or AGENTS_TOKEN) for prompt submission")?;
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
            let tok = token.ok_or("missing --token (or AGENTS_TOKEN) for workflows query")?;
            let workflows = provider.list_workflows(&tok).await?;
            println!("{}", serde_json::to_string_pretty(&workflows)?);
        }
        Commands::Autocron => {
            let tok = token.ok_or("missing --token (or AGENTS_TOKEN) for autocron query")?;
            let schedules = provider.list_autocron_schedules(&tok).await?;
            println!("{}", serde_json::to_string_pretty(&schedules)?);
        }
    }

    Ok(())
}
