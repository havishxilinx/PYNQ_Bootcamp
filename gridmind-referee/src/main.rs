use anyhow::Result;
use clap::{Parser, Subcommand};
use gridmind_referee::arena::run_arena;
use gridmind_referee::master::run_master;

#[derive(Parser)]
#[command(name = "gridmind-referee")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the tournament orchestrator.
    Master {
        #[arg(long)]
        server: String,
        #[arg(long, default_value = "bootcamp2024")]
        key: String,
        /// The join-listener thread derives its own board ID as
        /// "{id}-lobby" -- avoid an id that already ends in "-lobby".
        #[arg(long)]
        id: String,
        #[arg(long)]
        config: Option<String>,
        #[arg(long, default_value_t = 8080)]
        web_port: u16,
        /// Ignore any existing data/tournament_state.json for this run
        /// (without deleting it) and start registration completely fresh.
        #[arg(long)]
        fresh: bool,
    },
    /// Run the game engine for one arena. Waits for match assignments
    /// from the Master and plays them one after another.
    Arena {
        #[arg(long)]
        server: String,
        #[arg(long, default_value = "bootcamp2024")]
        key: String,
        #[arg(long)]
        id: String,
        #[arg(long)]
        master_id: String,
        #[arg(long)]
        arena_num: u32,
        /// Base URL of this arena's Genesis simulated-arm server, e.g.
        /// http://127.0.0.1:9005. Omit to run without Genesis entirely --
        /// purely cosmetic, the match plays identically either way.
        #[arg(long)]
        genesis_url: Option<String>,
        /// Admin password for this Genesis server's `admin_start_competition`/
        /// `admin_stop_competition` actions -- must match that server's own
        /// `GENESIS_ADMIN_PASSWORD` env var. Defaults to Genesis's own
        /// documented default; ignored entirely when `genesis_url` is unset.
        #[arg(long, default_value = "admin123")]
        genesis_admin_password: String,
        /// Port of this Genesis server's separate live-viewer/stream
        /// process (`stream_server.py`), on the same host as `genesis_url`
        /// but a different port -- must match that server's own
        /// `GENESIS_STREAM_PORT` env var. Defaults to Genesis's own
        /// documented default; ignored entirely when `genesis_url` is unset.
        #[arg(long, default_value_t = 8080)]
        genesis_stream_port: u16,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Master {
            server,
            key,
            id,
            config,
            web_port,
            fresh,
        } => {
            // Load config if provided; propagate any file/parse error immediately.
            let pools_config = config
                .map(|path| gridmind_referee::master::load_pools_config(&path))
                .transpose()?;

            let initial_state = match &pools_config {
                Some(cfg) => {
                    let pool1_names: Vec<String> = cfg
                        .pool1_teams
                        .iter()
                        .map(|(name, _)| name.clone())
                        .collect();
                    let pool2_names: Vec<String> = cfg
                        .pool2_teams
                        .iter()
                        .map(|(name, _)| name.clone())
                        .collect();
                    gridmind_referee::master::initial_scoreboard_state(&pool1_names, &pool2_names)
                }
                None => gridmind_referee::master::empty_registration_state(),
            };
            let master_state = gridmind_referee::master::MasterState::new(initial_state);
            let (operator_channels, rx) = gridmind_referee::master::operator_channels();
            let join_registry = gridmind_referee::join_registry::JoinRegistry::new();
            let team_secrets = gridmind_referee::team_secrets::TeamSecrets::new();
            let puzzle_answers = gridmind_referee::puzzle_answers::PuzzleAnswers::new();

            let app_state = gridmind_referee::web::AppState {
                master_state: master_state.clone(),
                operator_channels,
                join_registry: join_registry.clone(),
                puzzle_answers: puzzle_answers.clone(),
            };
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new()
                    .expect("failed to build tokio runtime for web server");
                if let Err(err) = rt.block_on(gridmind_referee::web::serve(web_port, app_state)) {
                    eprintln!("web server error: {err}");
                }
            });

            // Separate thread, separate board ID ("<id>-lobby") -- see
            // "Why join status is kept out of MasterState" in the Join
            // Competition design doc for why this can't share
            // run_master's own receive loop.
            {
                let lobby_id = format!("{id}-lobby");
                let lobby_client =
                    gridmind_referee::p2p_client::P2pClient::new(&server, &key, &lobby_id);
                let join_registry = join_registry.clone();
                let team_secrets = team_secrets.clone();
                let master_state = master_state.clone();
                std::thread::spawn(move || {
                    gridmind_referee::join_listener::run_join_listener(
                        lobby_client,
                        join_registry,
                        team_secrets,
                        master_state,
                    );
                });
            }

            run_master(
                &server,
                &key,
                &id,
                pools_config,
                master_state,
                rx,
                gridmind_referee::master::AuthState {
                    team_secrets,
                    join_registry,
                    puzzle_answers,
                    fresh,
                },
            )
        }
        Command::Arena {
            server,
            key,
            id,
            master_id,
            arena_num,
            genesis_url,
            genesis_admin_password,
            genesis_stream_port,
        } => run_arena(
            &server,
            &key,
            &id,
            &master_id,
            arena_num,
            gridmind_referee::arena::GenesisConfig {
                url: genesis_url,
                admin_password: genesis_admin_password,
                stream_port: genesis_stream_port,
            },
        ),
    }
}
