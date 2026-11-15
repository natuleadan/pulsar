use std::io;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::core::adapters::{NativeReqwestTransport, ServerModeProvider};
use crate::core::backend::{ControlAction, SandboxProvider};
use crate::core::state::{ActiveView, AppAction, AppState, AuthState, ClusterNodeEntry, dispatch};
use crate::tui::{TuiKey, map_key, render_tui};

/// Interactive async desktop TUI loop wired to `ServerModeProvider` over HTTP.
pub async fn run_live_tui(
    mut state: AppState,
    server_url: String,
    email: Option<String>,
    password: Option<String>,
    token: Option<String>,
) -> io::Result<()> {
    let transport = Arc::new(NativeReqwestTransport::default());
    let provider = ServerModeProvider::new(server_url.clone(), transport);
    dispatch(&mut state, AppAction::SetLoginUrl(server_url));

    let mut access_token = token.unwrap_or_default();
    if access_token.is_empty()
        && let (Some(em), Some(pw)) = (email.as_deref(), password.as_deref())
    {
        match provider.login(em, pw).await {
            Ok(pair) => {
                access_token = pair.access_token;
                dispatch(
                    &mut state,
                    AppAction::SetAuthState(AuthState::Authenticated, Some(em.to_owned())),
                );
                dispatch(&mut state, AppAction::SetView(ActiveView::Workspace));
            }
            Err(err) => {
                dispatch(&mut state, AppAction::SetError(Some(err.to_string())));
            }
        }
    } else if !access_token.is_empty() {
        dispatch(
            &mut state,
            AppAction::SetAuthState(AuthState::Authenticated, email),
        );
        dispatch(&mut state, AppAction::SetView(ActiveView::Workspace));
    }

    if !access_token.is_empty() {
        sync_server_snapshot(&provider, &access_token, &mut state).await;
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut last_seq = 0_u64;
    let res = loop {
        if let Err(err) = render_tui(&mut terminal, &state) {
            break Err(io::Error::other(err.to_string()));
        }

        if event::poll(Duration::from_millis(250))?
            && let Event::Key(key) = event::read()?
        {
            if key.code == KeyCode::Char('q') && state.composer_text.is_empty() {
                break Ok(());
            }
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                break Ok(());
            }

            match key.code {
                KeyCode::Backspace => {
                    let mut next = state.composer_text.clone();
                    next.pop();
                    dispatch(&mut state, AppAction::UpdateComposer(next));
                }
                KeyCode::Enter if !state.composer_text.trim().is_empty() => {
                    let prompt = state.composer_text.trim().to_owned();
                    dispatch(&mut state, AppAction::UpdateComposer(String::new()));
                    handle_composer_submit(&provider, &access_token, &mut state, &prompt).await;
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && state.active_view == ActiveView::Workspace =>
                {
                    let mut next = state.composer_text.clone();
                    next.push(c);
                    dispatch(&mut state, AppAction::UpdateComposer(next));
                }
                _ => {
                    let mapped = match key.code {
                        KeyCode::Tab => Some(TuiKey::Tab),
                        KeyCode::BackTab => Some(TuiKey::BackTab),
                        KeyCode::Up => Some(TuiKey::Up),
                        KeyCode::Down => Some(TuiKey::Down),
                        KeyCode::Left => Some(TuiKey::Left),
                        KeyCode::Right => Some(TuiKey::Right),
                        KeyCode::Enter => Some(TuiKey::Enter),
                        KeyCode::Esc => Some(TuiKey::Escape),
                        KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            Some(TuiKey::CtrlK)
                        }
                        KeyCode::Char(c) => Some(TuiKey::Char(c)),
                        _ => None,
                    };
                    if let Some(action) = mapped.and_then(|k| map_key(&state, k)) {
                        dispatch(&mut state, action);
                    }
                }
            }
        }

        if !access_token.is_empty()
            && let Some(session_id) = state.session_id.clone()
            && let Ok(events) = provider
                .replay_events(&access_token, &session_id, last_seq)
                .await
        {
            for ev in events {
                last_seq = last_seq.max(ev.seq);
                dispatch(
                    &mut state,
                    AppAction::AppendStreamChunk(format!("[{}] {:?}", ev.seq, ev.payload)),
                );
            }
        }
    };

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    res
}

async fn sync_server_snapshot(provider: &ServerModeProvider, token: &str, state: &mut AppState) {
    if let Ok(nodes) = provider.list_nodes(token).await {
        let mapped = nodes
            .into_iter()
            .map(|n| ClusterNodeEntry {
                node_id: n.node_id,
                vlan_ip: n.vlan_ip,
                runtime_label: "docker".to_owned(),
                active_boxes: n.active_boxes,
                max_boxes: n.max_boxes,
                status_label: n.status,
            })
            .collect();
        dispatch(state, AppAction::SetClusterNodes(mapped));
    }
    if state.session_id.is_none()
        && let Ok(sessions) = provider.list_sessions(token).await
        && let Some(first_id) = sessions.first().and_then(|v| v["id"].as_str())
    {
        dispatch(state, AppAction::SetSession(first_id.to_owned()));
    }
}

async fn handle_composer_submit(
    provider: &ServerModeProvider,
    token: &str,
    state: &mut AppState,
    prompt: &str,
) {
    if token.is_empty() {
        dispatch(
            state,
            AppAction::SetError(Some(
                "not authenticated; pass --token or --email/--password".to_owned(),
            )),
        );
        return;
    }

    if prompt == "/nodes" {
        sync_server_snapshot(provider, token, state).await;
        return;
    }

    if let Some(task_id) = prompt.strip_prefix("/cancel ") {
        let idem = format!("tui-cancel-{}", state.stream_output.len());
        match provider
            .send_control(token, task_id.trim(), ControlAction::Stop, &idem)
            .await
        {
            Ok(receipt) => dispatch(
                state,
                AppAction::AppendStreamChunk(format!("control: {}\n", receipt.status_note)),
            ),
            Err(err) => dispatch(state, AppAction::SetError(Some(err.to_string()))),
        }
        return;
    }

    let session_id = match state.session_id.clone() {
        Some(id) => id,
        None => {
            let mode_str = serde_json::to_value(state.interaction_mode)
                .ok()
                .and_then(|v| v.as_str().map(ToOwned::to_owned))
                .unwrap_or_else(|| "code".to_owned());
            match provider
                .create_session(token, "TUI Session", &mode_str)
                .await
            {
                Ok(val) => {
                    let id = val["id"]
                        .as_str()
                        .or_else(|| val["session"]["id"].as_str())
                        .unwrap_or("default")
                        .to_owned();
                    dispatch(state, AppAction::SetSession(id.clone()));
                    id
                }
                Err(err) => {
                    dispatch(state, AppAction::SetError(Some(err.to_string())));
                    return;
                }
            }
        }
    };

    match provider
        .submit_task(token, &session_id, prompt, state.interaction_mode)
        .await
    {
        Ok(sub) => {
            dispatch(
                state,
                AppAction::AppendStreamChunk(format!("> {} (task {})\n", sub.prompt, sub.task_id)),
            );
        }
        Err(err) => dispatch(state, AppAction::SetError(Some(err.to_string()))),
    }
}
