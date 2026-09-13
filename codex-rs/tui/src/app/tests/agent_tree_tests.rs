use super::*;
use crate::app::agent_tree::AgentPickerLayout;
use crate::app::agent_tree::agent_tree_rows;
use pretty_assertions::assert_eq;

#[test]
fn agent_tree_groups_descendants_and_keeps_sibling_spawn_order() {
    let ids: Vec<_> = (0..6).map(|_| ThreadId::new()).collect();
    let mut state = AgentNavigationState::default();
    // A descendant may be discovered before its parent during a background refresh.
    for (id, path) in ids.iter().zip([
        "/root/work/review",
        "/root/worker",
        "/root/work",
        "/root",
        "/root/work/test",
        "/root/missing/deep",
    ]) {
        state.record_sub_agent_activity(SubAgentActivityDisplay {
            thread_id: *id,
            agent_path: path.to_string(),
            is_running_hint: true,
        });
    }
    let rows = agent_tree_rows(state.ordered_threads(), Some(ids[3]));
    assert_eq!(
        rows.into_iter()
            .map(|row| (row.thread_id, row.prefix))
            .collect::<Vec<_>>(),
        vec![
            (ids[3], String::new()),
            (ids[1], "├── ".to_string()),
            (ids[2], "├── ".to_string()),
            (ids[0], "│   ├── ".to_string()),
            (ids[4], "│   └── ".to_string()),
            (ids[5], "└── ".to_string()),
        ]
    );
}

#[tokio::test]
async fn agent_tree_shows_statuses_and_opens_selected_transcript() {
    let (mut app, mut events, _ops) = make_test_app_with_channels().await;
    let root = ThreadId::from_string("00000000-0000-0000-0000-000000000100").unwrap();
    let worker = ThreadId::from_string("00000000-0000-0000-0000-000000000101").unwrap();
    let review = ThreadId::from_string("00000000-0000-0000-0000-000000000102").unwrap();
    let done = ThreadId::from_string("00000000-0000-0000-0000-000000000103").unwrap();
    app.primary_thread_id = Some(root);
    app.active_thread_id = Some(review);
    for (id, path) in [
        (root, "/root"),
        (worker, "/root/worker"),
        (review, "/root/worker/review"),
        (done, "/root/done"),
    ] {
        app.agent_navigation
            .record_sub_agent_activity(SubAgentActivityDisplay {
                thread_id: id,
                agent_path: path.to_string(),
                is_running_hint: true,
            });
    }
    app.agent_navigation.mark_stopped(review);
    app.agent_navigation.mark_closed(done);
    app.agent_navigation.picker_layout = AgentPickerLayout::Tree;
    let params = app.agent_picker_selection_view_params(/*selected*/ None);
    assert_eq!(params.initial_selected_idx, Some(2));
    app.chat_widget.show_selection_view(params);
    insta::assert_snapshot!(
        "agent_tree",
        render_bottom_popup(&app.chat_widget, /*width*/ 90)
    );
    insta::assert_snapshot!(
        "agent_tree_narrow",
        render_bottom_popup(&app.chat_widget, /*width*/ 48)
    );
    while events.try_recv().is_ok() {}
    app.chat_widget.handle_key_event(KeyCode::Enter.into());
    assert_matches!(events.try_recv(), Ok(AppEvent::SelectAgentThread(id)) if id == review);
}

#[test]
fn agent_tree_keeps_legacy_and_rootless_threads_inspectable() {
    let root = ThreadId::new();
    let legacy = ThreadId::new();
    let mut state = AgentNavigationState::default();
    state.upsert(
        legacy,
        Some("Worker".to_string()),
        /*agent_role*/ None,
        /*is_closed*/ false,
    );
    state.upsert(
        root, /*agent_nickname*/ None, /*agent_role*/ None, /*is_closed*/ false,
    );
    let rows = agent_tree_rows(state.ordered_threads(), Some(root));
    assert_eq!(
        rows.into_iter()
            .map(|row| (row.thread_id, row.prefix))
            .collect::<Vec<_>>(),
        vec![(root, String::new()), (legacy, "└── ".to_string())]
    );
    let rows = agent_tree_rows(state.ordered_threads(), /*primary_thread_id*/ None);
    assert_eq!(
        rows.into_iter()
            .map(|row| row.thread_id)
            .collect::<Vec<_>>(),
        vec![legacy, root]
    );
}

#[tokio::test]
async fn agent_tree_refresh_preserves_selected_thread_when_parent_is_discovered() {
    let (mut app, mut events, _ops) = make_test_app_with_channels().await;
    let root = ThreadId::new();
    let child = ThreadId::new();
    let sibling = ThreadId::new();
    let parent = ThreadId::new();
    app.primary_thread_id = Some(root);
    app.active_thread_id = Some(sibling);
    app.agent_navigation.picker_layout = AgentPickerLayout::Tree;
    for (id, path) in [
        (root, "/root"),
        (child, "/root/parent/child"),
        (sibling, "/root/sibling"),
    ] {
        app.agent_navigation
            .record_sub_agent_activity(SubAgentActivityDisplay {
                thread_id: id,
                agent_path: path.to_string(),
                is_running_hint: true,
            });
    }
    let params = app.agent_picker_selection_view_params(/*selected*/ None);
    app.chat_widget.show_selection_view(params);
    let mut thread: codex_app_server_protocol::Thread = serde_json::from_value(serde_json::json!({
        "id": parent.to_string(), "sessionId": root.to_string(), "preview": "parent",
        "ephemeral": false, "modelProvider": "openai", "createdAt": 0, "updatedAt": 0,
        "status": {"type": "idle"}, "cwd": app.config.cwd, "cliVersion": "0.0.0",
        "source": "cli", "turns": []
    }))
    .unwrap();
    thread.source = codex_app_server_protocol::SessionSource::SubAgent(
        SubAgentSource::ThreadSpawn {
            parent_thread_id: root,
            depth: 1,
            agent_path: Some("/root/parent".parse().unwrap()),
            agent_nickname: None,
            agent_role: None,
        },
    );
    let request = app.agent_navigation.begin_picker_refresh(root).unwrap();
    app.apply_agent_picker_thread_refresh(root, request, Ok(vec![thread]));
    while events.try_recv().is_ok() {}
    app.chat_widget.handle_key_event(KeyCode::Enter.into());
    assert_matches!(events.try_recv(), Ok(AppEvent::SelectAgentThread(id)) if id == sibling);
}

#[tokio::test]
async fn agent_tree_empty_state_does_not_prompt_to_enable_subagents() -> Result<()> {
    let (mut app, mut events, _ops) = make_test_app_with_channels().await;
    let mut app_server = start_config_write_test_app_server(&app).await?;
    let mut tui = crate::tui::test_support::make_test_tui()?;
    app.primary_thread_id =
        Some(ThreadId::from_string("00000000-0000-0000-0000-000000000100").unwrap());
    while events.try_recv().is_ok() {}
    app.handle_event(&mut tui, &mut app_server, AppEvent::OpenAgentTree)
        .await?;
    insta::assert_snapshot!(
        "agent_tree_empty",
        render_bottom_popup(&app.chat_widget, /*width*/ 80)
    );
    assert_eq!(app.agent_navigation.picker_layout, AgentPickerLayout::Tree);
    app_server.shutdown().await?;
    Ok(())
}
