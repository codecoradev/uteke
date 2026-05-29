//! Uteke CLI — persistent memory for AI agents.

use clap::Parser;

#[derive(Parser)]
#[command(name = "uteke", about = "The Brain for Your AI")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Store a memory
    Remember {
        /// The content to remember
        content: String,
        /// Tags for categorization
        #[arg(long, value_delimiter = ',')]
        tags: Vec<String>,
    },
    /// Recall relevant memories
    Recall {
        /// The search query
        query: String,
        /// Maximum results
        #[arg(long, default_value = "5")]
        limit: usize,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Some(Commands::Remember { content, tags }) => {
            println!("Remembering: {content} (tags: {:?})", tags);
        }
        Some(Commands::Recall { query, limit }) => {
            println!("Recalling '{query}' (limit: {limit})");
            println!("(not yet implemented)");
        }
        None => {
            println!("Use uteke --help for available commands.");
        }
    }
}
