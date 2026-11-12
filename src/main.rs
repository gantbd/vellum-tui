#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use crossterm::{
        event::{self, DisableBracketedPaste, EnableBracketedPaste, Event},
        execute,
    };
    use std::{io::stdout, path::PathBuf};
    use vellum_adapter_clipboard_macos::{DesktopAccess, IntegrationError, ReferenceClipboard};
    use vellum_model::{
        Action, CommentDraft, CommentHandle, CommentHistoryDirection, CommentPointProfile,
        DestinationPlan, Editor, EditorError, SearchDirection,
    };
    use vellum_tui::{
        Input, PromptInput, TextPrompt, destination_notice, draw, draw_comments,
        plan_workspace_roots, translate_key,
    };
    enum Entry {
        Open,
        Find,
        Replace(bool),
        Destination(bool),
        Comment(CommentDraft),
        EditComment(CommentHandle),
        CommentFile(bool),
    }

    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|argument| argument == "--service") {
        return run_service_mode(&args);
    }
    if args.len() == 1 && args[0] == "--help" {
        println!(
            "Ctrl-C/X/V copies/cuts/pastes source through the native AppKit adapter; Ctrl-Q quits. F12 retries retained delivery only, never provider I/O. Binding is lazy; live clipboard qualification remains pending."
        );
        println!(
            "vellum-tui [EXISTING_FILE_OR_DIRECTORY...]\nWith no arguments, starts empty. Ctrl-O adds a file or directory (4096-byte frontend entry limit); Ctrl-S save; Ctrl-Z/Y undo/redo; arrows/Home/End; F4 editing lock; F6 next file; Ctrl-Q quit.\nCtrl-F literal find; F3/Shift-F3 next/previous; Ctrl-R replace current; F8 replace all complete results. Empty replacement deletes; Esc cancels. Search again after edits.\nUnsaved files require Ctrl-Q again to discard. Editing lock does not change OS permissions.\nReference profile: macOS local files, 64 KiB/file, at most 16 discovered files, LF for generated newlines, native Unicode 18 source movement/deletion (ICU4C 79.1 host prerequisite). Terminal-width tables and form input remain separate. Query/replacement entry limits: 1024/65536 bytes.\nMouse, live OS clipboard, IME preedit, bidi layout and screen-reader behavior are not qualified."
        );
        println!(
            "Service mode: vellum-tui --service SOCKET [--credential CREDENTIAL]. It uses the shared workspace client, polls revision notifications, and supports witnessed text editing, save, document cycling, and explicit 'f PATH' / 'd PATH' admission through Ctrl-O."
        );
        println!(
            "F2 Save As; F7 export copy. Existing destination only: enter its literal path, then Y confirms overwrite; Esc cancels. Save As retains at most 16 prior binding entries in this reference application."
        );
        println!(
            "Shift-arrows/Home/End extend selection; Ctrl-A selects all. Typing, bracketed paste, Backspace or Delete replace the selection. Plain Left/Right collapse it."
        );
        println!(
            "F5 indents selected/current lines; Shift-Tab outdents using native four-column semantics. Tab remains literal insertion."
        );
        println!(
            "F9 selects complete current literal matches (native reference limit: 16 input selections). Typing/paste, deletion, movement and indentation act on every native selection. Partial/stale/over-budget results refuse; no automatic query refresh."
        );
        println!(
            "F10 comments each selection (empty targets: content points); Shift-F10 chooses position points. Multiline entry: Enter LF, Ctrl-S commits, Esc cancels. F11 lists/edits/deletes comments, U/R history, L locking, S/O JSON save/load. Initial JSON destination requires explicit Y confirmation. Source save does not persist comments; unsaved comments require explicit discard."
        );
        return Ok(());
    }
    let roots: Vec<PathBuf> = args.into_iter().map(PathBuf::from).collect();
    let Some(paths) = (if roots.is_empty() {
        choose_initial_paths()
    } else {
        Ok(Some(plan_workspace_roots(&roots)?))
    })?
    else {
        return Ok(());
    };
    let mut editor = Editor::open(&paths, vellum_tui::reference_session_config())?;
    editor.enable_native_unicode(
        vellum_adapter_unicode_icu::IcuFactory::reference(),
        vellum_model::NativeUnicodeProfile::reference(),
    )?;
    let mut clipboard = ReferenceClipboard::new(&mut editor)?;
    editor.enable_comment_files(vellum_model::CommentFileLimits::reference())?;
    ratatui::run(|terminal| -> Result<(), Box<dyn std::error::Error>> {
        struct PasteGuard;
        impl Drop for PasteGuard {
            fn drop(&mut self) {
                let _ = execute!(stdout(), DisableBracketedPaste);
            }
        }
        let _paste_guard = PasteGuard;
        execute!(stdout(), EnableBracketedPaste)?;
        let mut confirming = false;
        let mut notice: Option<String> = None;
        let mut opening: Option<(TextPrompt, Entry)> = None;
        let mut destination: Option<DestinationPlan> = None;
        let mut comment_destination: Option<vellum_model::CommentFilePlan> = None;
        let mut show_comments = false;
        let mut selected_comment = 0usize;
        loop {
            if notice.is_none()
                && clipboard.stage() != vellum_adapter_clipboard_macos::ClipboardStage::Idle
            {
                notice = Some("Clipboard pending; F12 retries delivery only. Discard cannot erase unresolved work.".into());
            }
            let size = terminal.size()?;
            if show_comments && opening.is_none() && comment_destination.is_none() {
                let locked = editor.comments_locked();
                let dirty = match editor.comments_dirty() {
                    Ok(dirty) => dirty,
                    Err(error) => {
                        notice = Some(error.to_string());
                        show_comments = false;
                        continue;
                    }
                };
                let rows = match editor.comments() {
                    Ok(rows) => rows,
                    Err(error) => {
                        notice = Some(error.to_string());
                        show_comments = false;
                        continue;
                    }
                };
                selected_comment = selected_comment.min(rows.len().saturating_sub(1));
                terminal.draw(|frame| {
                    draw_comments(
                        frame,
                        rows,
                        selected_comment,
                        locked,
                        dirty,
                        notice.as_deref(),
                    )
                })?;
                if let Event::Key(key) = event::read()? {
                    use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
                    if key.kind == KeyEventKind::Release {
                        continue;
                    }
                    match key.code {
                        KeyCode::Char('s' | 'o')
                            if key.modifiers == KeyModifiers::NONE
                                && key.kind == KeyEventKind::Press =>
                        {
                            let load = key.code == KeyCode::Char('o');
                            if editor.comment_file_bound() {
                                let result = if load {
                                    editor.load_comment_file()
                                } else {
                                    editor.save_comment_file()
                                };
                                notice = Some(match result {
                                    Ok(()) => editor.status().into(),
                                    Err(error) => error.to_string(),
                                });
                            } else {
                                opening = Some((
                                    TextPrompt::new(
                                        4096,
                                        if load {
                                            "Load existing annotation JSON"
                                        } else {
                                            "Save comments to existing file"
                                        },
                                    ),
                                    Entry::CommentFile(load),
                                ));
                                notice = None;
                            }
                        }
                        KeyCode::Char('d')
                            if key.modifiers == KeyModifiers::NONE
                                && key.kind == KeyEventKind::Press =>
                        {
                            notice=Some(match editor.retire_failed_comment_read(){Ok(_)=>"Failed read released; unactivated JSON still protected from Save".into(),Err(e)=>e.to_string()});
                        }
                        KeyCode::Esc | KeyCode::F(11) => {
                            show_comments = false;
                            confirming = false;
                            notice = None;
                        }
                        KeyCode::Up => selected_comment = selected_comment.saturating_sub(1),
                        KeyCode::Down => {
                            selected_comment = selected_comment
                                .saturating_add(1)
                                .min(rows.len().saturating_sub(1))
                        }
                        KeyCode::Enter if key.kind == KeyEventKind::Press => {
                            if let Some(row) = rows.get(selected_comment) {
                                let handle = row.handle.clone();
                                let initial = row.body.clone();
                                match TextPrompt::multiline(
                                    editor.comment_body_limit().unwrap_or(0),
                                    "Edit session comment",
                                    &initial,
                                ) {
                                    Ok(prompt) => {
                                        opening = Some((prompt, Entry::EditComment(handle)));
                                        notice = None;
                                    }
                                    Err(error) => notice = Some(error.to_string()),
                                }
                            }
                        }
                        KeyCode::Delete if key.kind == KeyEventKind::Press => {
                            if let Some(handle) =
                                rows.get(selected_comment).map(|r| r.handle.clone())
                            {
                                notice =
                                    editor.delete_comment(&handle).err().map(|e| e.to_string());
                            }
                        }
                        KeyCode::Char('a' | 'p')
                            if key.modifiers == KeyModifiers::NONE
                                && key.kind == KeyEventKind::Press =>
                        {
                            if let Some(handle) =
                                rows.get(selected_comment).map(|r| r.handle.clone())
                            {
                                notice = editor
                                    .begin_comment(if key.code == KeyCode::Char('p') {
                                        CommentPointProfile::Position
                                    } else {
                                        CommentPointProfile::Content
                                    })
                                    .and_then(|draft| editor.reattach_comment(&handle, &draft))
                                    .err()
                                    .map(|e| e.to_string());
                            }
                        }
                        KeyCode::Char('u' | 'r')
                            if key.modifiers == KeyModifiers::NONE
                                && key.kind == KeyEventKind::Press =>
                        {
                            notice = editor
                                .reverse_comment(if key.code == KeyCode::Char('u') {
                                    CommentHistoryDirection::Undo
                                } else {
                                    CommentHistoryDirection::Redo
                                })
                                .err()
                                .map(|e| e.to_string());
                        }
                        KeyCode::Char('l')
                            if key.modifiers == KeyModifiers::NONE
                                && key.kind == KeyEventKind::Press =>
                        {
                            notice = editor
                                .set_comments_locked(!locked)
                                .err()
                                .map(|e| e.to_string());
                        }
                        _ if matches!(translate_key(key), Input::Quit) => {
                            match clipboard.close(&mut editor, confirming) {
                                Ok(()) => break,
                                Err(IntegrationError::Editor(EditorError::Unsaved)) => {
                                    confirming = true;
                                    notice = Some("Unsaved source/session comments. Ctrl-Q again discards all; Esc cancels.".into());
                                }
                                Err(error) => notice = Some(error.to_string()),
                            }
                        }
                        _ => {}
                    }
                }
                continue;
            }
            let mut view =
                editor.frame(size.width.saturating_sub(2), size.height.saturating_sub(4))?;
            if opening.is_some() || destination.is_some() || comment_destination.is_some() {
                view.cursor = None;
            }
            let prompt = opening.as_ref().map(|(p, _)| match &notice {
                Some(error) => format!(
                    "{error} | {}",
                    p.display(usize::from(size.width).saturating_sub(error.len() + 3))
                ),
                None => p.display(usize::from(size.width)),
            });
            let confirmation = destination
                .as_ref()
                .map(|plan| destination_notice(plan, size.width));
            let comment_confirmation = comment_destination.as_ref().map(|p| {
                let escaped: String = p
                    .destination()
                    .to_string_lossy()
                    .chars()
                    .flat_map(char::escape_default)
                    .take(usize::from(size.width).saturating_sub(38))
                    .collect();
                format!(
                    "Y confirms JSON {} / Esc cancels: {escaped}",
                    if p.is_load() { "load" } else { "overwrite" }
                )
            });
            terminal.draw(|frame| {
                draw(
                    frame,
                    &view,
                    comment_confirmation
                        .as_deref()
                        .or(confirmation.as_deref())
                        .or(prompt.as_deref())
                        .or(notice.as_deref()),
                )
            })?;
            let event = event::read()?;
            if let Some(plan) = comment_destination.take() {
                use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
                match event {
                    Event::Key(key)
                        if key.kind == KeyEventKind::Press
                            && key.code == KeyCode::Char('y')
                            && key.modifiers == KeyModifiers::NONE =>
                    {
                        notice = Some(match editor.confirm_comment_file(&plan) {
                            Ok(()) => editor.status().into(),
                            Err(e) => e.to_string(),
                        });
                    }
                    Event::Key(key) if key.code == KeyCode::Esc => {
                        editor.cancel_comment_file_plan();
                        notice = None;
                    }
                    _ => comment_destination = Some(plan),
                }
                continue;
            }
            if let Some(plan) = destination.take() {
                match event {
                    Event::Key(key)
                        if key.kind == crossterm::event::KeyEventKind::Press
                            && key.code == crossterm::event::KeyCode::Char('y')
                            && key.modifiers == crossterm::event::KeyModifiers::NONE =>
                    {
                        notice = editor
                            .confirm_destination(&plan)
                            .err()
                            .map(|e| e.to_string());
                    }
                    Event::Key(key)
                        if key.code == crossterm::event::KeyCode::Esc
                            || key.code == crossterm::event::KeyCode::Char('c')
                                && key.modifiers == crossterm::event::KeyModifiers::CONTROL =>
                    {
                        notice = None;
                    }
                    _ => destination = Some(plan),
                }
                continue;
            }
            if let Some((prompt, mode)) = &mut opening {
                let entered = matches!(&event, Event::Key(_) | Event::Paste(_));
                let input = match event {
                    Event::Key(key) => prompt.key(key),
                    Event::Paste(text) => prompt.insert(&text).map(|()| PromptInput::Pending),
                    _ => Ok(PromptInput::Pending),
                };
                match input {
                    Ok(PromptInput::Cancel) => {
                        opening = None;
                        notice = None;
                    }
                    Ok(PromptInput::Submit(text)) => match match mode {
                        Entry::Open => plan_workspace_roots([PathBuf::from(text)])
                            .map_err(|error| error.to_string())
                            .and_then(|paths| {
                                editor
                                    .open_additional_many(&paths)
                                    .map(|_| ())
                                    .map_err(|error| error.to_string())
                            }),
                        Entry::Find => editor.find(&text).map_err(|error| error.to_string()),
                        Entry::Replace(all) => editor
                            .replace_match(&text, *all)
                            .map_err(|error| error.to_string()),
                        Entry::Destination(save_as) => {
                            match editor.plan_destination(PathBuf::from(text), *save_as) {
                                Ok(plan) => {
                                    destination = Some(plan);
                                    Ok(())
                                }
                                Err(error) => Err(error.to_string()),
                            }
                        }
                        Entry::Comment(draft) => editor
                            .create_comments(draft, &text)
                            .map(|_| ())
                            .map_err(|error| error.to_string()),
                        Entry::EditComment(handle) => editor
                            .update_comment(handle, &text)
                            .map_err(|error| error.to_string()),
                        Entry::CommentFile(load) => editor
                            .plan_comment_file(PathBuf::from(text), *load)
                            .map(|p| {
                                comment_destination = Some(p);
                            })
                            .map_err(|error| error.to_string()),
                    } {
                        Ok(()) => {
                            if matches!(mode, Entry::Comment(_) | Entry::EditComment(_)) {
                                show_comments = true;
                                selected_comment = 0;
                            }
                            opening = None;
                            notice = destination.as_ref().map(|p| {
                                format!(
                                    "Y overwrites existing file ({}) / Esc cancels",
                                    if p.is_save_as() {
                                        "Save As"
                                    } else {
                                        "copy export"
                                    }
                                )
                            });
                        }
                        Err(error) => notice = Some(error),
                    },
                    Ok(PromptInput::Pending) => {
                        if entered {
                            notice = None;
                        }
                    }
                    Err(error) => notice = Some(error.to_string()),
                }
                continue;
            }
            match event {
                Event::Key(key) => match translate_key(key) {
                    Input::Quit => match clipboard.close(&mut editor, confirming) {
                        Ok(()) => break,
                        Err(IntegrationError::Editor(EditorError::Unsaved)) => {
                            confirming = true;
                            notice = Some(
                                "Unsaved source/session comments. Ctrl-Q again discards all; Esc cancels.".into(),
                            );
                        }
                        Err(error) => {
                            notice = Some(error.to_string());
                        }
                    },
                    Input::Clipboard(operation) => {
                        confirming = false;
                        // Explicit trusted local-owner reference-host premises;
                        // native source locks/currentness remain independently enforced.
                        let access = DesktopAccess {
                            control: true,
                            source_read: true,
                            source_edit: true,
                            clipboard_read: true,
                            clipboard_write: true,
                            retention: true,
                        };
                        notice = Some(
                            match operation {
                                Some(operation) => {
                                    clipboard.perform(&mut editor, operation, access)
                                }
                                None => clipboard.resume(&mut editor, access),
                            }
                            .map_or_else(|e| e.to_string(), |r| r.notice()),
                        );
                    }
                    Input::CancelQuit => {
                        confirming = false;
                        notice = None;
                    }
                    Input::Open => {
                        confirming = false;
                        notice = None;
                        opening = Some((TextPrompt::new(4096, "Open"), Entry::Open));
                    }
                    Input::Find => {
                        confirming = false;
                        notice = None;
                        opening = Some((TextPrompt::new(1024, "Find literal"), Entry::Find));
                    }
                    Input::Replace(all) => {
                        confirming = false;
                        notice = None;
                        opening = Some((
                            TextPrompt::new(
                                65536,
                                if all {
                                    "Replace all"
                                } else {
                                    "Replace current"
                                },
                            ),
                            Entry::Replace(all),
                        ));
                    }
                    Input::Match(previous) => {
                        confirming = false;
                        notice = editor
                            .navigate_match(
                                if previous {
                                    SearchDirection::Previous
                                } else {
                                    SearchDirection::Next
                                },
                                true,
                            )
                            .err()
                            .map(|e| e.to_string());
                    }
                    Input::SelectMatches => {
                        confirming = false;
                        notice = editor.select_matches().err().map(|e| e.to_string());
                    }
                    Input::Comment(position) => {
                        confirming = false;
                        match editor.begin_comment(if position {
                            CommentPointProfile::Position
                        } else {
                            CommentPointProfile::Content
                        }) {
                            Ok(draft) => match TextPrompt::multiline(
                                editor.comment_body_limit().unwrap_or(0),
                                if position {
                                    "Add position-point comment(s)"
                                } else {
                                    "Add content comment(s)"
                                },
                                "",
                            ) {
                                Ok(prompt) => {
                                    opening = Some((prompt, Entry::Comment(draft)));
                                    notice = None;
                                }
                                Err(error) => notice = Some(error.to_string()),
                            },
                            Err(error) => notice = Some(error.to_string()),
                        }
                    }
                    Input::Comments => {
                        confirming = false;
                        show_comments = true;
                        selected_comment = 0;
                        notice = None;
                    }
                    Input::Destination(save_as) => {
                        confirming = false;
                        notice = None;
                        opening = Some((
                            TextPrompt::new(
                                4096,
                                if save_as {
                                    "Save As existing file"
                                } else {
                                    "Export copy to existing file"
                                },
                            ),
                            Entry::Destination(save_as),
                        ));
                    }
                    Input::Edit(action) => {
                        confirming = false;
                        notice = None;
                        // The model preserves the precise error and original save
                        // task. Refusal never becomes a successful write or retry.
                        let _ = editor.apply(action);
                    }
                    Input::Ignore => {}
                },
                Event::Paste(text) => {
                    confirming = false;
                    notice = None;
                    let _ = editor.apply(Action::Insert(text));
                }
                _ => {}
            }
        }
        Ok(())
    })
}

#[cfg(target_os = "macos")]
fn choose_initial_paths() -> Result<Option<Vec<std::path::PathBuf>>, Box<dyn std::error::Error>> {
    use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
    use vellum_tui::{PromptInput, TextPrompt, draw_empty, plan_workspace_roots};

    ratatui::run(|terminal| {
        let mut prompt: Option<TextPrompt> = None;
        let mut notice: Option<String> = None;
        loop {
            let size = terminal.size()?;
            let prompt_notice = prompt.as_ref().map(|entry| match &notice {
                Some(error) => format!(
                    "{error} | {}",
                    entry.display(usize::from(size.width).saturating_sub(error.len() + 3))
                ),
                None => entry.display(usize::from(size.width)),
            });
            terminal.draw(|frame| {
                draw_empty(frame, prompt_notice.as_deref().or(notice.as_deref()));
            })?;
            match event::read()? {
                Event::Key(key)
                    if key.kind == KeyEventKind::Press
                        && key.modifiers == KeyModifiers::CONTROL
                        && key.code == KeyCode::Char('q') =>
                {
                    return Ok(None);
                }
                Event::Key(key)
                    if prompt.is_none()
                        && key.kind == KeyEventKind::Press
                        && key.modifiers == KeyModifiers::CONTROL
                        && key.code == KeyCode::Char('o') =>
                {
                    prompt = Some(TextPrompt::new(4096, "Open file or directory"));
                    notice = None;
                }
                Event::Key(key) if prompt.is_some() => {
                    let result = prompt.as_mut().expect("checked above").key(key);
                    match result {
                        Ok(PromptInput::Submit(root)) => {
                            match plan_workspace_roots([std::path::PathBuf::from(root)]) {
                                Ok(paths) => return Ok(Some(paths)),
                                Err(error) => notice = Some(error.to_string()),
                            }
                            prompt = None;
                        }
                        Ok(PromptInput::Cancel) => {
                            prompt = None;
                            notice = None;
                        }
                        Ok(PromptInput::Pending) => notice = None,
                        Err(error) => notice = Some(error.to_string()),
                    }
                }
                Event::Paste(text) if prompt.is_some() => {
                    if let Err(error) = prompt.as_mut().expect("checked above").insert(&text) {
                        notice = Some(error.to_string());
                    }
                }
                _ => {}
            }
        }
    })
}

#[cfg(target_os = "macos")]
enum ServiceConnector {
    Plain(vellum_sdk_client::transport::UnixConnector),
    Authenticated(vellum_sdk_client::transport::AuthenticatedUnixConnector),
}

#[cfg(target_os = "macos")]
impl vellum_sdk_client::transport::FrameExchange for ServiceConnector {
    fn exchange(
        &mut self,
        request: &[u8],
        max_reply_bytes: usize,
    ) -> Result<Vec<u8>, vellum_sdk_client::transport::TransportError> {
        match self {
            Self::Plain(connector) => connector.exchange(request, max_reply_bytes),
            Self::Authenticated(connector) => connector.exchange(request, max_reply_bytes),
        }
    }
}

#[cfg(target_os = "macos")]
fn run_service_mode(args: &[std::ffi::OsString]) -> Result<(), Box<dyn std::error::Error>> {
    use crossterm::{
        event::{self, DisableBracketedPaste, EnableBracketedPaste, Event},
        execute,
    };
    use std::{io::stdout, path::PathBuf, time::Duration};
    use vellum_sdk_client::{
        transport::{AuthenticatedUnixConnector, RemoteTransport, TransportLimits, UnixConnector},
        workspace::{PersistenceProjectionV1, WorkspaceClientV1},
    };
    use vellum_tui::{
        Input, PromptInput, ServiceAction, ServiceEditor, TextPrompt, draw, draw_empty,
        translate_key,
    };

    let (socket, credential) = match args {
        [service, socket] if service == "--service" => (PathBuf::from(socket), None),
        [service, socket, flag, credential] if service == "--service" && flag == "--credential" => {
            (PathBuf::from(socket), Some(PathBuf::from(credential)))
        }
        _ => {
            return Err(std::io::Error::other(
                "usage: vellum-tui --service SOCKET [--credential CREDENTIAL]",
            )
            .into());
        }
    };
    let connector = if let Some(credential) = credential {
        let mut store = vellum_adapter_local_authentication::LocalCredentialStoreV1::default();
        let record = store
            .load(&credential)
            .map_err(|error| std::io::Error::other(format!("credential refused: {error:?}")))?;
        let provider =
            vellum_adapter_local_authentication::LocalCredentialClaimProviderV1::new(record);
        ServiceConnector::Authenticated(AuthenticatedUnixConnector::with_provider(socket, provider))
    } else {
        ServiceConnector::Plain(UnixConnector::new(socket))
    };
    let transport = RemoteTransport::new(connector, TransportLimits::reference())
        .map_err(|error| std::io::Error::other(format!("transport refused: {error:?}")))?;
    let workspace = WorkspaceClientV1::connect(transport)
        .map_err(|error| std::io::Error::other(format!("service refused: {error:?}")))?;
    let editor = ServiceEditor::open(workspace)?;
    let editor = ratatui::run(
        |terminal| -> Result<ServiceEditor<_>, Box<dyn std::error::Error>> {
            struct PasteGuard;
            impl Drop for PasteGuard {
                fn drop(&mut self) {
                    let _ = execute!(stdout(), DisableBracketedPaste);
                }
            }
            let _paste_guard = PasteGuard;
            execute!(stdout(), EnableBracketedPaste)?;
            let mut editor = editor;
            let mut prompt: Option<TextPrompt> = None;
            let mut notice: Option<String> = None;
            let mut confirming_discard = false;
            loop {
                if let Err(error) = editor.poll() {
                    notice = Some(error.to_string());
                }
                let size = terminal.size()?;
                let prompt_notice = prompt.as_ref().map(|entry| match &notice {
                    Some(error) => format!(
                        "{error} | {}",
                        entry.display(usize::from(size.width).saturating_sub(error.len() + 3))
                    ),
                    None => entry.display(usize::from(size.width)),
                });
                if let Some(frame) = editor.frame(size.height.saturating_sub(4)) {
                    terminal.draw(|terminal_frame| {
                        draw(
                            terminal_frame,
                            &frame,
                            prompt_notice.as_deref().or(notice.as_deref()),
                        )
                    })?;
                } else {
                    terminal.draw(|terminal_frame| {
                        draw_empty(
                            terminal_frame,
                            prompt_notice.as_deref().or(notice.as_deref()),
                        )
                    })?;
                }
                if !event::poll(Duration::from_millis(250))? {
                    continue;
                }
                let event = event::read()?;
                if let Some(entry) = prompt.as_mut() {
                    match event {
                        Event::Key(key) => match entry.key(key) {
                            Ok(PromptInput::Submit(input)) => {
                                let result = input
                                    .strip_prefix("f ")
                                    .map(|path| editor.admit_file(path.to_owned()))
                                    .or_else(|| {
                                        input
                                            .strip_prefix("d ")
                                            .map(|path| editor.admit_directory(path.to_owned()))
                                    });
                                notice = Some(match result {
                                    Some(Ok(())) => "Workspace membership updated".into(),
                                    Some(Err(error)) => error.to_string(),
                                    None => "Use 'f PATH' or 'd PATH'".into(),
                                });
                                prompt = None;
                            }
                            Ok(PromptInput::Cancel) => {
                                prompt = None;
                                notice = None;
                            }
                            Ok(PromptInput::Pending) => notice = None,
                            Err(error) => notice = Some(error.to_string()),
                        },
                        Event::Paste(text) => {
                            if let Err(error) = entry.insert(&text) {
                                notice = Some(error.to_string());
                            }
                        }
                        _ => {}
                    }
                    continue;
                }
                if let Event::Paste(text) = event {
                    notice = editor
                        .apply(ServiceAction::Insert(text))
                        .err()
                        .map(|error| error.to_string());
                    continue;
                }
                let Event::Key(key) = event else {
                    continue;
                };
                match translate_key(key) {
                    Input::Quit => {
                        let persistence = editor.workspace().projection().persistence();
                        if persistence == PersistenceProjectionV1::Indeterminate {
                            notice = Some(
                                "An edit/save outcome is indeterminate; repeat the exact action before exit"
                                    .into(),
                            );
                        } else if persistence == PersistenceProjectionV1::Dirty
                            && !confirming_discard
                        {
                            confirming_discard = true;
                            notice = Some(
                                "Unsaved service edit; Ctrl-Q again discards, Esc cancels".into(),
                            );
                        } else {
                            break;
                        }
                    }
                    Input::CancelQuit => {
                        confirming_discard = false;
                        notice = None;
                    }
                    Input::Open => {
                        prompt = Some(TextPrompt::new(4096, "Add: f PATH or d PATH"));
                        notice = None;
                    }
                    Input::Edit(action) => {
                        let action = match action {
                            vellum_model::Action::Insert(text) => Some(ServiceAction::Insert(text)),
                            vellum_model::Action::Backspace => Some(ServiceAction::Backspace),
                            vellum_model::Action::Delete => Some(ServiceAction::Delete),
                            vellum_model::Action::Left => Some(ServiceAction::Left),
                            vellum_model::Action::Right => Some(ServiceAction::Right),
                            vellum_model::Action::Up => Some(ServiceAction::Up),
                            vellum_model::Action::Down => Some(ServiceAction::Down),
                            vellum_model::Action::Home => Some(ServiceAction::Home),
                            vellum_model::Action::End => Some(ServiceAction::End),
                            vellum_model::Action::Save => Some(ServiceAction::Save),
                            vellum_model::Action::NextDocument => Some(ServiceAction::NextDocument),
                            _ => None,
                        };
                        notice = match action {
                            Some(action) => {
                                editor.apply(action).err().map(|error| error.to_string())
                            }
                            None => {
                                Some("This command is not yet available in service mode".into())
                            }
                        };
                        confirming_discard = false;
                    }
                    _ => {
                        notice = Some("This command is not yet available in service mode".into());
                    }
                }
            }
            Ok(editor)
        },
    )?;
    editor
        .disconnect()
        .map_err(|failure| std::io::Error::other(format!("disconnect refused: {failure:?}")))?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("vellum currently requires the macOS adapter profile");
    std::process::exit(1);
}
