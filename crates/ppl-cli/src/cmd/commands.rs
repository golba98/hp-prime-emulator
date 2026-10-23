//! `ppl commands`: the built-in command table.

use clap::Args;
use ppl_compiler::builtins::{BUILTINS, Status};

use super::common::exit;

#[derive(Args, Debug)]
pub struct CommandsOpts {
    /// Only show this category (math, strings, lists, matrices, graphics, io, …).
    #[arg(long)]
    pub category: Option<String>,
    /// Only show commands with this status (implemented, approximate, unsupported).
    #[arg(long)]
    pub status: Option<String>,
    /// Look up one command.
    pub name: Option<String>,
}

pub fn list(o: CommandsOpts) -> u8 {
    let mut shown = 0;
    println!("{:<14} {:<10} {:<9} status", "command", "category", "args");
    for s in BUILTINS {
        if o.category
            .as_deref()
            .is_some_and(|c| !c.eq_ignore_ascii_case(s.category.as_str()))
        {
            continue;
        }
        if o.status
            .as_deref()
            .is_some_and(|st| !st.eq_ignore_ascii_case(s.status.as_str()))
        {
            continue;
        }
        if o.name
            .as_deref()
            .is_some_and(|n| !n.eq_ignore_ascii_case(s.name))
        {
            continue;
        }
        println!(
            "{:<14} {:<10} {:<9} {}",
            s.name,
            s.category.as_str(),
            s.arity_text(),
            s.status.as_str()
        );
        shown += 1;
    }
    let unsupported = BUILTINS
        .iter()
        .filter(|s| s.status == Status::Unsupported)
        .count();
    eprintln!(
        "{shown} shown; {} commands known, {unsupported} unsupported. No command is hardware-verified yet; see docs/compatibility/matrix.md.",
        BUILTINS.len()
    );
    if shown == 0 { exit::USAGE } else { exit::OK }
}
