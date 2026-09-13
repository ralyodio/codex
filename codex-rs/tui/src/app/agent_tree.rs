//! Hierarchical presentation of the existing root-scoped subagent picker.
//!
//! Canonical agent paths supply ancestry without additional backend requests. Siblings keep their
//! first-seen order; missing ancestors and legacy agents remain inspectable under the main thread.

use super::agent_picker::AGENT_PICKER_VIEW_ID;
use super::*;
use crate::multi_agents::AgentPickerThreadEntry;
use std::collections::HashMap;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum AgentPickerLayout {
    #[default]
    List,
    Tree,
}

pub(super) struct AgentTreeRow<'a> {
    pub(super) thread_id: ThreadId,
    entry: &'a AgentPickerThreadEntry,
    pub(super) prefix: String,
}

pub(super) fn agent_tree_rows(
    threads: Vec<(ThreadId, &AgentPickerThreadEntry)>,
    primary_thread_id: Option<ThreadId>,
) -> Vec<AgentTreeRow<'_>> {
    let primary = threads
        .iter()
        .position(|(id, _)| Some(*id) == primary_thread_id);
    let paths: HashMap<_, _> = threads
        .iter()
        .enumerate()
        .filter_map(|(index, (_, entry))| {
            entry
                .agent_path
                .as_deref()
                .map(|path| (path.trim_end_matches('/'), index))
        })
        .collect();
    let mut children = vec![Vec::new(); threads.len()];
    let mut roots = Vec::new();
    for (index, (_, entry)) in threads.iter().enumerate() {
        if Some(index) == primary {
            roots.insert(0, index);
            continue;
        }
        let mut path = entry
            .agent_path
            .as_deref()
            .unwrap_or("")
            .trim_end_matches('/');
        let mut parent = primary;
        while let Some((ancestor, _)) = path.rsplit_once('/') {
            if let Some(ancestor_index) = paths.get(ancestor) {
                parent = Some(*ancestor_index);
                break;
            }
            path = ancestor;
        }
        if let Some(parent) = parent {
            children[parent].push(index);
        } else {
            roots.push(index);
        }
    }

    let mut rows = Vec::with_capacity(threads.len());
    let mut pending: Vec<_> = roots
        .into_iter()
        .rev()
        .map(|index| (index, String::new(), String::new()))
        .collect();
    while let Some((index, prefix, child_prefix)) = pending.pop() {
        let (thread_id, entry) = threads[index];
        rows.push(AgentTreeRow {
            thread_id,
            entry,
            prefix,
        });
        for (position, child) in children[index].iter().enumerate().rev() {
            let last = position + 1 == children[index].len();
            let branch = if last { "└── " } else { "├── " };
            let continuation = if last { "    " } else { "│   " };
            pending.push((
                *child,
                format!("{child_prefix}{branch}"),
                format!("{child_prefix}{continuation}"),
            ));
        }
    }
    rows
}

impl App {
    pub(super) fn agent_tree_selection_view_params(
        &self,
        selected: Option<usize>,
    ) -> SelectionViewParams {
        let rows = agent_tree_rows(
            self.agent_navigation.ordered_threads(),
            self.primary_thread_id,
        );
        let subtitle = if rows
            .iter()
            .all(|row| Some(row.thread_id) == self.primary_thread_id)
        {
            "No subagents yet. Ask Codex to delegate a task, then reopen /tree."
        } else {
            "This session's subagents. Reopen /tree to refresh."
        };
        let initial_selected_idx = selected.or_else(|| {
            rows.iter()
                .position(|row| Some(row.thread_id) == self.active_thread_id)
        });
        let items = rows
            .into_iter()
            .map(|row| {
                let id = row.thread_id;
                let entry = row.entry;
                let is_primary = self.primary_thread_id == Some(id);
                let path = entry
                    .agent_path
                    .as_deref()
                    .unwrap_or("")
                    .trim_end_matches('/');
                let name = if !is_primary && !path.is_empty() {
                    path.rsplit('/').next().unwrap_or(path).to_string()
                } else {
                    format_agent_picker_item_name(
                        entry.agent_nickname.as_deref(),
                        entry.agent_role.as_deref(),
                        is_primary,
                    )
                };
                let (status, dot) = if entry.is_closed {
                    ("Closed", "• ".dim())
                } else if entry.is_running {
                    ("Running", "• ".green())
                } else {
                    ("Idle", "• ".dim())
                };
                let detail = if path.is_empty() {
                    id.to_string()
                } else {
                    path.to_string()
                };
                SelectionItem {
                    name: name.clone(),
                    name_prefix_spans: vec![row.prefix.dim(), dot],
                    description: Some(format!("{status} · {detail}")),
                    is_current: self.active_thread_id == Some(id),
                    actions: vec![Box::new(move |tx| tx.send(AppEvent::SelectAgentThread(id)))],
                    dismiss_on_select: true,
                    search_value: Some(format!("{name} {path} {status} {id}")),
                    ..Default::default()
                }
            })
            .collect();
        SelectionViewParams {
            view_id: Some(AGENT_PICKER_VIEW_ID),
            title: Some("Agent tree".to_string()),
            subtitle: Some(subtitle.to_string()),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            initial_selected_idx,
            ..Default::default()
        }
    }
}
