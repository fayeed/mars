//! The Mars MCP server as its own executable, for development and for agents
//! configured by hand. The desktop app serves the same thing as `mars-desktop
//! mcp ...`, so nothing extra has to ship with it.

fn main() {
    std::process::exit(mars_mcp::run_from_args(std::env::args().skip(1)));
}
