mod agents;
mod app;
mod command;
mod config;
mod import;
mod mcp;
mod platform;
mod routing;
mod state;
mod sync;

use config::Config;

const HELP: &str = "valerie: a fast, configurable, agent-native browser

USAGE:
  valerie [URL...]        open the browser (URLs open like external links; forwarded if already running)
  valerie mcp-config      print the command that connects Claude Code to the built-in MCP server
  valerie mcp-stdio       stdio <-> HTTP bridge for MCP clients that only speak stdio
  valerie sync-setup      create a private GitHub repo for tab sync (needs the gh CLI)
  valerie default-config  print every config option with its default
  valerie paths           show config and data locations";

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config = Config::load().unwrap_or_else(|e| {
        eprintln!("valerie: config error, using defaults: {e:#}");
        Config::defaults()
    });
    match args.first().map(String::as_str) {
        Some("-h" | "--help" | "help") => {
            println!("{HELP}");
            return Ok(());
        }
        Some("mcp-stdio") => return mcp::stdio_bridge(config.mcp.port),
        Some("sync-setup") => return sync::setup(),
        Some("mcp-config") => {
            let token = mcp::load_or_create_token();
            println!(
                "claude mcp add --transport http valerie http://127.0.0.1:{}/mcp --header \"Authorization: Bearer {token}\"",
                config.mcp.port
            );
            return Ok(());
        }
        Some("default-config") => {
            print!("{}", config::DEFAULT_YAML);
            return Ok(());
        }
        Some("paths") => {
            println!("config: {}\ndata:   {}", config::config_path().display(), config::data_dir().display());
            return Ok(());
        }
        _ => {}
    }
    let urls: Vec<String> = args.into_iter().filter(|a| !a.starts_with('-')).collect();
    if !urls.is_empty() && config.mcp.enabled && mcp::forward_open(config.mcp.port, &urls) {
        return Ok(());
    }
    app::run(config, urls)
}
