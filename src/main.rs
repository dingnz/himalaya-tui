// This file is part of Himalaya TUI, a TUI to manage emails.
//
// Copyright (C) 2025-2026 soywod <pimalaya.org@posteo.net>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU Affero General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option) any
// later version.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE. See the GNU Affero General Public License for more
// details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

use std::{fs::File, io};

use anyhow::Result;
use clap::Parser;
use himalaya_tui::{
    cli::HimalayaTui,
    runtime::{events, startup},
};
use pimalaya_cli::printer::StdoutPrinter;
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    crossterm::{
        event::{DisableMouseCapture, EnableMouseCapture},
        execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    },
};

fn main() -> Result<()> {
    let cli = HimalayaTui::parse();

    // Auxiliary subcommands (completions, manuals) run before the TUI
    // ever starts and print to stdout.
    if let Some(command) = cli.command {
        let mut printer = StdoutPrinter::new(&cli.json);
        return command.execute(&mut printer);
    }

    let log_file = File::create("/tmp/himalaya-tui.log")?;
    simplelog::WriteLogger::init(
        simplelog::LevelFilter::Trace,
        simplelog::Config::default(),
        log_file,
    )?;

    // Resolve config (loaded from disk if present, otherwise built
    // in-memory by the wizard) in normal terminal mode so inquire
    // prompts can render. The TUI's alternate screen kicks in after
    // the client is built.
    let (mut app, client) = match startup::load_then_connect(
        &cli.config_paths,
        cli.account.name.as_deref(),
        cli.no_config,
        cli.from.as_deref(),
        cli.keybinds,
    ) {
        Ok(setup) => setup,
        Err(err) => {
            eprintln!("Error: {err:?}");
            return Ok(());
        }
    };

    if let Some(from) = cli.from {
        app.from = Some(from);
    }
    if let Some(from_name) = cli.from_name {
        app.from_name = Some(from_name);
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let result = events::run(&mut terminal, app, client);

    let _ = disable_raw_mode();
    let _ = execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    );
    let _ = terminal.show_cursor();

    if let Err(err) = result {
        eprintln!("Error: {err:?}");
    }

    Ok(())
}
