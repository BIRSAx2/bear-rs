use anyhow::{Result, anyhow};
use clap::Parser;

use crate::bear::{join_tags, maybe_push, maybe_push_bool, open_bear_action};
use crate::cli::Cli;
use crate::cli::Commands;
use crate::config::{encode_file, load_token, resolve_database_path, save_token};
use crate::db::BearDb;

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    let db = match &cli.command {
        Commands::OpenNote(_)
        | Commands::Tags
        | Commands::OpenTag(_)
        | Commands::Search(_)
        | Commands::Untagged(_)
        | Commands::Todo(_)
        | Commands::Today(_)
        | Commands::Locked(_) => Some(BearDb::open(resolve_database_path(
            cli.database.as_deref(),
        )?)?),
        _ => None,
    };

    match cli.command {
        Commands::Auth(cmd) => {
            save_token(&cmd.token)?;
            println!("Saved API token.");
        }
        Commands::OpenNote(cmd) => {
            let note = db
                .as_ref()
                .expect("db available for read command")
                .find_note(cmd.id.as_deref(), cmd.title.as_deref(), cmd.exclude_trashed)?;
            println!("{}", note.text);
        }
        Commands::Tags => {
            for tag in db.as_ref().expect("db available for read command").tags()? {
                println!("{tag}");
            }
        }
        Commands::OpenTag(cmd) => {
            for note in db
                .as_ref()
                .expect("db available for read command")
                .notes_for_tags(&split_csv(&cmd.name), false)?
            {
                println!("{}\t{}", note.identifier, note.title);
            }
        }
        Commands::Search(cmd) => {
            for note in db.as_ref().expect("db available for read command").search(
                cmd.term.as_deref(),
                cmd.tag.as_deref(),
                false,
            )? {
                println!("{}\t{}", note.identifier, note.title);
            }
        }
        Commands::Untagged(cmd) => {
            for note in db
                .as_ref()
                .expect("db available for read command")
                .untagged(cmd.search.as_deref())?
            {
                println!("{}\t{}", note.identifier, note.title);
            }
        }
        Commands::Todo(cmd) => {
            for note in db
                .as_ref()
                .expect("db available for read command")
                .todo(cmd.search.as_deref())?
            {
                println!("{}\t{}", note.identifier, note.title);
            }
        }
        Commands::Today(cmd) => {
            for note in db
                .as_ref()
                .expect("db available for read command")
                .today(cmd.search.as_deref())?
            {
                println!("{}\t{}", note.identifier, note.title);
            }
        }
        Commands::Locked(cmd) => {
            for note in db
                .as_ref()
                .expect("db available for read command")
                .locked(cmd.search.as_deref())?
            {
                println!("{}\t{}", note.identifier, note.title);
            }
        }
        Commands::Create(cmd) => {
            let mut query = Vec::new();
            maybe_push(&mut query, "title", cmd.title);
            maybe_push(&mut query, "text", cmd.text);
            maybe_push(&mut query, "tags", join_tags(&cmd.tag));
            maybe_push_bool(&mut query, "open_note", cmd.open_note);
            maybe_push_bool(&mut query, "new_window", cmd.new_window);
            maybe_push_bool(&mut query, "float", cmd.float);
            maybe_push_bool(&mut query, "show_window", cmd.show_window);
            maybe_push_bool(&mut query, "pin", cmd.pin);
            maybe_push_bool(&mut query, "edit", cmd.edit);
            maybe_push_bool(&mut query, "timestamp", cmd.timestamp);
            maybe_push(&mut query, "type", cmd.kind);
            maybe_push(&mut query, "url", cmd.url);

            if let Some(file) = cmd.file {
                let filename = cmd
                    .filename
                    .or_else(|| {
                        file.file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                    })
                    .ok_or_else(|| {
                        anyhow!("--filename is required when the file path has no file name")
                    })?;
                let encoded = encode_file(&file)?;
                maybe_push(&mut query, "filename", Some(filename));
                maybe_push(&mut query, "file", Some(encoded));
            }

            open_bear_action("create", &query)?;
        }
        Commands::AddText(cmd) => {
            let mut query = Vec::new();
            maybe_push(&mut query, "id", cmd.id);
            maybe_push(&mut query, "title", cmd.title);
            maybe_push(&mut query, "text", cmd.text);
            maybe_push(&mut query, "header", cmd.header);
            maybe_push(&mut query, "mode", Some(cmd.mode));
            maybe_push(&mut query, "tags", join_tags(&cmd.tag));
            maybe_push_bool(&mut query, "exclude_trashed", cmd.exclude_trashed);
            maybe_push_bool(&mut query, "new_line", cmd.new_line);
            maybe_push_bool(&mut query, "open_note", cmd.open_note);
            maybe_push_bool(&mut query, "new_window", cmd.new_window);
            maybe_push_bool(&mut query, "show_window", cmd.show_window);
            maybe_push_bool(&mut query, "edit", cmd.edit);
            maybe_push_bool(&mut query, "timestamp", cmd.timestamp);
            open_bear_action("add-text", &query)?;
        }
        Commands::AddFile(cmd) => {
            let mut query = Vec::new();
            maybe_push(&mut query, "id", cmd.id);
            maybe_push(&mut query, "title", cmd.title);
            maybe_push(&mut query, "header", cmd.header);
            maybe_push(&mut query, "mode", Some(cmd.mode));
            maybe_push_bool(&mut query, "open_note", cmd.open_note);
            maybe_push_bool(&mut query, "new_window", cmd.new_window);
            maybe_push_bool(&mut query, "show_window", cmd.show_window);
            maybe_push_bool(&mut query, "edit", cmd.edit);
            maybe_push(
                &mut query,
                "filename",
                cmd.filename.or_else(|| {
                    cmd.file
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                }),
            );
            maybe_push(&mut query, "file", Some(encode_file(&cmd.file)?));
            open_bear_action("add-file", &query)?;
        }
        Commands::GrabUrl(cmd) => {
            let mut query = Vec::new();
            maybe_push(&mut query, "url", Some(cmd.url));
            maybe_push(&mut query, "tags", join_tags(&cmd.tag));
            maybe_push_bool(&mut query, "pin", cmd.pin);
            maybe_push_bool(&mut query, "wait", cmd.wait);
            open_bear_action("grab-url", &query)?;
        }
        Commands::Trash(cmd) => {
            let mut query = Vec::new();
            maybe_push(&mut query, "id", cmd.id);
            maybe_push(&mut query, "search", cmd.search);
            maybe_push_bool(&mut query, "show_window", cmd.show_window);
            open_bear_action("trash", &query)?;
        }
        Commands::Archive(cmd) => {
            let mut query = Vec::new();
            maybe_push(&mut query, "id", cmd.id);
            maybe_push(&mut query, "search", cmd.search);
            maybe_push_bool(&mut query, "show_window", cmd.show_window);
            open_bear_action("archive", &query)?;
        }
        Commands::RenameTag(cmd) => {
            let mut query = Vec::new();
            maybe_push(&mut query, "name", Some(cmd.name));
            maybe_push(&mut query, "new_name", Some(cmd.new_name));
            maybe_push_bool(&mut query, "show_window", cmd.show_window);
            open_bear_action("rename-tag", &query)?;
        }
        Commands::DeleteTag(cmd) => {
            let mut query = Vec::new();
            maybe_push(&mut query, "name", Some(cmd.name));
            maybe_push_bool(&mut query, "show_window", cmd.show_window);
            open_bear_action("delete-tag", &query)?;
        }
        Commands::Raw(cmd) => {
            let mut params = cmd.params;
            let has_token = params.iter().any(|(key, _)| key == "token");
            if !has_token {
                if let Some(token) = cmd.token {
                    params.push(("token".into(), token));
                } else if cmd.use_saved_token {
                    if let Some(token) = load_token()? {
                        params.push(("token".into(), token));
                    }
                }
            }
            open_bear_action(&cmd.action, &params)?;
        }
    }

    Ok(())
}

fn split_csv(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}
