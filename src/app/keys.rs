//! Keyboard and mouse input: the main key handler (tabs, lists and every
//! popup), cursor movement and click handling.

use super::*;

impl App {
    pub fn handle_key(&mut self, code: KeyCode) -> bool {
        // Help overlay handles its own dismiss
        if self.show_help {
            match code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Char('?') | KeyCode::F(1) => {
                    self.show_help = false;
                }
                KeyCode::Up | KeyCode::Char('k') => self.help_scroll = self.help_scroll.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => self.help_scroll = self.help_scroll.saturating_add(1),
                KeyCode::PageUp => self.help_scroll = self.help_scroll.saturating_sub(10),
                KeyCode::PageDown => self.help_scroll = self.help_scroll.saturating_add(10),
                KeyCode::Home => self.help_scroll = 0,
                KeyCode::End => self.help_scroll = u16::MAX,
                _ => {}
            }
            return false;
        }

        // Config editor is a global overlay — handle it early regardless of tab
        if matches!(self.detail_popup, Some(DetailPopup::ConfigEditor(_))) {
            return self.handle_config_editor_key(code);
        }

        // Save-to-file prompt is a global overlay too, same reasoning
        if self.save_prompt.is_some() {
            return self.handle_save_prompt_key(code);
        }

        // Sniffer input mode (text entry before launching, only on Events tab)
        if self.sniffer_input_active && self.current_tab == Tab::Events {
            match code {
                KeyCode::Esc => {
                    self.sniffer_input_active = false;
                    self.sniffer_input_text.clear();
                }
                KeyCode::Enter => {
                    let ids: Vec<String> = self.sniffer_input_text
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    if !ids.is_empty() {
                        let sniffer_max = self.config.sniffer_buffer_size;
                        // Color occurrences of the just-entered minion
                        // target(s) within captured content (glob-resolved,
                        // same as the Events tab's own filter terms) --
                        // purely cosmetic, not a second filter on top of
                        // `minion_ids`'s own broader relevance matching. See
                        // `SnifferState::color_terms` doc for why: a
                        // wildcard-targeted job relevant to a sniffed minion
                        // doesn't literally contain that minion's name
                        // anywhere in its JSON, so re-filtering display by
                        // literal text match on these same terms would hide
                        // legitimate, already-captured traffic.
                        let color_terms = self.resolve_host_terms(ids.iter().map(|s| s.as_str()));
                        self.sniffer = Some(crate::types::SnifferState {
                            minion_ids: ids.clone(),
                            buffer: std::collections::VecDeque::with_capacity(sniffer_max),
                            max_size: sniffer_max,
                            paused: false,
                            selected: 0,
                            following: true,
                            color_terms,
                            highlight_keywords: Vec::new(),
                            highlight_input: None,
                        });
                    }
                    self.sniffer_input_active = false;
                    self.sniffer_input_text.clear();
                }
                KeyCode::Backspace => {
                    self.sniffer_input_text.pop();
                }
                KeyCode::Char(c) => {
                    self.sniffer_input_text.push(c);
                }
                _ => {}
            }
            return false;
        }

        // Detail popup gets first crack
        let mut clipboard_text: Option<String> = None;
        // Sibling of clipboard_text for 's'/'S' ("save to file"): (content, filename hint)
        // (content, filename hint, extension)
        let mut save_text: Option<(String, String, &str)> = None;
        let had_popup = self.detail_popup.is_some() && self.popup_tab == Some(self.current_tab);
        // The sniffer lives outside `detail_popup` (see its field doc) so it
        // keeps running when you're elsewhere, but it still owns the Events
        // tab's key input whenever it's the thing on screen there.
        let sniffer_showing = self.current_tab == Tab::Events && self.detail_popup.is_none() && self.sniffer.is_some();

        // Contrast toggle and tab switching work universally (even with popups open)
        // but are muted when a text input mode is active inside the popup
        if had_popup || sniffer_showing {
            let popup_text_input = if sniffer_showing {
                self.sniffer.as_ref().is_some_and(|s| s.highlight_input.is_some())
            } else {
                matches!(&self.detail_popup, Some(DetailPopup::JobDetail { return_is_filtering: true, .. }))
            };
            if !popup_text_input {
                match code {
                    KeyCode::Tab => { self.current_tab = self.current_tab.next(); return false; }
                    KeyCode::BackTab => { self.current_tab = self.current_tab.prev(); return false; }
                    KeyCode::Char('1') => { self.current_tab = Tab::Dashboard; return false; }
                    KeyCode::Char('2') => { self.current_tab = Tab::Minions; return false; }
                    KeyCode::Char('3') => { self.current_tab = Tab::Events; return false; }
                    KeyCode::Char('4') => { self.current_tab = Tab::Jobs; return false; }
                    KeyCode::Char('5') => { self.current_tab = Tab::Alerts; return false; }
                    KeyCode::Char('6') => { self.current_tab = Tab::Api; return false; }
                    KeyCode::Char('7') => { self.current_tab = Tab::Pulse; return false; }
                    KeyCode::F(3) => { self.open_config_editor(); return false; }
                    KeyCode::F(2) => {
                        self.mouse_enabled = !self.mouse_enabled;
                        self.needs_mouse_toggle = true;
                        return false;
                    }
                    KeyCode::Char('?') | KeyCode::F(1) => {
                        self.show_help = true;
                        self.help_scroll = 0;
                        return false;
                    }
                    _ => {}
                }
            }
        }

        // Local variable to defer jid_lookup agent command (avoids borrow conflict)
        let mut trigger_y_lookup: Option<String> = None;
        let mut trigger_grains_fetch: Option<String> = None;
        let mut trigger_event_pretty_lookup: Option<(String, String)> = None;
        // Pre-compute events for MinionEvents popup to avoid borrow conflict
        let mut minion_events_cache: Option<(String, Vec<MinionActivityItem>)> = None;
        if let Some(DetailPopup::MinionEvents { ref minion_id, .. }) = self.detail_popup {
            let mut items = self.minion_activity(minion_id);
            // Apply filter if set
            if !self.minion_events_filter.is_empty() {
                let pat = self.minion_events_filter.to_lowercase();
                items.retain(|item| matches_filter(item, &pat));
            }
            if self.popup_tab == Some(self.current_tab) {
                minion_events_cache = Some((minion_id.clone(), items));
            }
        }
        if had_popup {
            if let Some(ref mut detail) = self.detail_popup {
            match *detail {
                DetailPopup::JobDetail {
                    ref mut selected_return,
                    ref mut return_filter,
                    ref mut return_is_filtering,
                    ref mut panel_focus,
                    ref mut content_scroll,
                    ref mut content_scroll_mode,
                    ref lookup,
                    ref jid,
                    ..
                } => {
                    // Return filter input mode
                    if *return_is_filtering {
                        match code {
                            KeyCode::Esc => {
                                *return_is_filtering = false;
                                return_filter.clear();
                            }
                            KeyCode::Enter => {
                                *return_is_filtering = false;
                                // Snap selection to nearest matching entry
                                if !return_filter.is_empty() {
                                    if let Some(ref l) = lookup {
                                        let cur = selected_return.unwrap_or(0);
                                        // Try forward from current position
                                        let next = (cur..l.returns.len())
                                            .find(|&i| l.returns[i].minion_id.contains(&*return_filter));
                                        if let Some(idx) = next {
                                            *selected_return = Some(idx);
                                        } else {
                                            // Try backward from current position
                                            let prev = (0..cur).rev()
                                                .find(|&i| l.returns[i].minion_id.contains(&*return_filter));
                                            if let Some(idx) = prev {
                                                *selected_return = Some(idx);
                                            }
                                        }
                                    }
                                }
                            }
                            KeyCode::Backspace => { return_filter.pop(); }
                            KeyCode::Char(c) => { return_filter.push(c); }
                            _ => {}
                        }
                        return false;
                    }
                    // Right panel (content preview) focused
                    if *panel_focus {
                        // self.job_detail_scroll_max is the true viewport-aware
                        // last-scroll-position for the return preview, recomputed
                        // and written back every render (see draw_job_detail) —
                        // clamping against it (instead of growing unbounded) is
                        // what keeps End from scrolling past the last line and
                        // PageUp responsive right afterward.
                        if *content_scroll_mode {
                            match code {
                                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                                    *content_scroll_mode = false;
                                }
                                KeyCode::Up | KeyCode::Char('k') => {
                                    *content_scroll = content_scroll.saturating_sub(1);
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    *content_scroll = (*content_scroll + 1).min(self.job_detail_scroll_max);
                                }
                                KeyCode::PageUp => {
                                    *content_scroll = content_scroll.saturating_sub(10);
                                }
                                KeyCode::PageDown => {
                                    *content_scroll = (*content_scroll + 10).min(self.job_detail_scroll_max);
                                }
                                KeyCode::Home => { *content_scroll = 0; }
                                KeyCode::End => { *content_scroll = self.job_detail_scroll_max; }
                                _ => {}
                            }
                            return false;
                        }
                        match code {
                            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                                *panel_focus = false;
                            }
                            KeyCode::Enter => {
                                *content_scroll_mode = true;
                                *content_scroll = 0;
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                *content_scroll = (*content_scroll + 1).min(self.job_detail_scroll_max);
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                *content_scroll = content_scroll.saturating_sub(1);
                            }
                            KeyCode::PageUp => {
                                *content_scroll = content_scroll.saturating_sub(10);
                            }
                            KeyCode::PageDown => {
                                *content_scroll = (*content_scroll + 10).min(self.job_detail_scroll_max);
                            }
                            KeyCode::Home => { *content_scroll = 0; }
                            KeyCode::End => { *content_scroll = self.job_detail_scroll_max; }
                            KeyCode::Char('y') => {
                                if let Some(ref l) = lookup {
                                    let idx = selected_return.unwrap_or(0);
                                    if idx < l.returns.len() {
                                        let ret = &l.returns[idx];
                                        clipboard_text = Some(
                                            if !ret.formatted_text.is_empty() {
                                                Self::strip_ansi(&ret.formatted_text)
                                            } else {
                                                Self::format_return_as_console(&ret.minion_id, &ret.full_return)
                                            }
                                        );
                                    }
                                }
                            }
                            KeyCode::Char('Y') => {
                                if let Some(ref l) = lookup {
                                    let idx = selected_return.unwrap_or(0);
                                    if idx < l.returns.len() {
                                        clipboard_text = Some(
                                            serde_json::to_string_pretty(&l.returns[idx].full_return)
                                                .unwrap_or_else(|_| l.returns[idx].summary.clone())
                                        );
                                    }
                                }
                                trigger_y_lookup = Some(jid.clone());
                            }
                            KeyCode::Char('s') => {
                                if let Some(ref l) = lookup {
                                    let idx = selected_return.unwrap_or(0);
                                    if idx < l.returns.len() {
                                        let ret = &l.returns[idx];
                                        let text = if !ret.formatted_text.is_empty() {
                                            Self::strip_ansi(&ret.formatted_text)
                                        } else {
                                            Self::format_return_as_console(&ret.minion_id, &ret.full_return)
                                        };
                                        save_text = Some((text, format!("job-{}-{}", jid, ret.minion_id), "txt"));
                                    }
                                }
                            }
                            KeyCode::Char('S') => {
                                if let Some(ref l) = lookup {
                                    let idx = selected_return.unwrap_or(0);
                                    if idx < l.returns.len() {
                                        let json = serde_json::to_string_pretty(&l.returns[idx].full_return)
                                            .unwrap_or_else(|_| l.returns[idx].summary.clone());
                                        save_text = Some((json, format!("job-{}-{}", jid, l.returns[idx].minion_id), "json"));
                                    }
                                }
                            }
                            _ => {}
                        }
                        // Don't return false — let clipboard_text/save_text flow through
                    } else {
                    // Left panel (return list) focused
                    match code {
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                            if let Some((mid, sel)) = self.return_to_minion_events.take() {
                                self.detail_popup = Some(DetailPopup::MinionEvents {
                                    minion_id: mid, selected: sel,
                                });
                            } else {
                                self.detail_popup = None;
                            }
                        }
                        KeyCode::Enter => {
                            if lookup.is_some() {
                                *panel_focus = true;
                                *content_scroll = 0;
                                *content_scroll_mode = false;
                            }
                        }
                        KeyCode::Char('/') => {
                            *return_is_filtering = true;
                            return_filter.clear();
                        }
                        KeyCode::Up | KeyCode::Char('k') | KeyCode::Down | KeyCode::Char('j')
                        | KeyCode::PageUp | KeyCode::PageDown | KeyCode::Home | KeyCode::End => {
                            if let Some(ref l) = lookup {
                                let visible: Vec<usize> = (0..l.returns.len())
                                    .filter(|&i| return_filter.is_empty() || l.returns[i].minion_id.contains(&*return_filter))
                                    .collect();
                                if !visible.is_empty() {
                                    let cur = selected_return.unwrap_or(0);
                                    let pos = visible.iter().position(|&i| i >= cur).unwrap_or(visible.len() - 1);
                                    let last = visible.len() - 1;
                                    let new_pos = match code {
                                        KeyCode::Up | KeyCode::Char('k') => pos.saturating_sub(1),
                                        KeyCode::Down | KeyCode::Char('j') => (pos + 1).min(last),
                                        KeyCode::PageUp => pos.saturating_sub(10),
                                        KeyCode::PageDown => (pos + 10).min(last),
                                        KeyCode::Home => 0,
                                        _ => last,
                                    };
                                    *selected_return = Some(visible[new_pos]);
                                }
                            }
                        }
                        KeyCode::Char('y') => {
                            if let Some(ref l) = lookup {
                                let idx = selected_return.unwrap_or(0);
                                if idx < l.returns.len() {
                                    let ret = &l.returns[idx];
                                    clipboard_text = Some(
                                        if !ret.formatted_text.is_empty() {
                                            Self::strip_ansi(&ret.formatted_text)
                                        } else {
                                            Self::format_return_as_console(&ret.minion_id, &ret.full_return)
                                        }
                                    );
                                }
                            }
                        }
                        KeyCode::Char('Y') => {
                            if let Some(ref l) = lookup {
                                let idx = selected_return.unwrap_or(0);
                                if idx < l.returns.len() {
                                    clipboard_text = Some(
                                        serde_json::to_string_pretty(&l.returns[idx].full_return)
                                            .unwrap_or_else(|_| l.returns[idx].summary.clone())
                                    );
                                }
                            }
                            trigger_y_lookup = Some(jid.clone());
                        }
                        KeyCode::Char('s') => {
                            if let Some(ref l) = lookup {
                                let idx = selected_return.unwrap_or(0);
                                if idx < l.returns.len() {
                                    let ret = &l.returns[idx];
                                    let text = if !ret.formatted_text.is_empty() {
                                        Self::strip_ansi(&ret.formatted_text)
                                    } else {
                                        Self::format_return_as_console(&ret.minion_id, &ret.full_return)
                                    };
                                    save_text = Some((text, format!("job-{}-{}", jid, ret.minion_id), "txt"));
                                }
                            }
                        }
                        KeyCode::Char('S') => {
                            if let Some(ref l) = lookup {
                                let idx = selected_return.unwrap_or(0);
                                if idx < l.returns.len() {
                                    let json = serde_json::to_string_pretty(&l.returns[idx].full_return)
                                        .unwrap_or_else(|_| l.returns[idx].summary.clone());
                                    save_text = Some((json, format!("job-{}-{}", jid, l.returns[idx].minion_id), "json"));
                                }
                            }
                        }
                        KeyCode::Char('a') => {
                            if let Some(ref l) = lookup {
                                if !l.returns.is_empty() {
                                    let text = Self::format_all_returns_console(jid, l);
                                    save_text = Some((text, format!("job-{}-all", jid), "txt"));
                                }
                            }
                        }
                        KeyCode::Char('A') => {
                            if let Some(ref l) = lookup {
                                if !l.returns.is_empty() {
                                    let text = Self::format_all_returns_json(l);
                                    save_text = Some((text, format!("job-{}-all", jid), "json"));
                                }
                            }
                        }
                        _ => {}
                    }
                }
                }
                DetailPopup::AdminConfirm => {
                    match code {
                        KeyCode::Enter => {
                            self.admin_mode = true;
                            self.admin_mode_pending = false;
                            self.detail_popup = self.return_to_minion_detail.take()
                                .map(|mid| DetailPopup::Minion(mid, PingState::Idle, None, 0));
                        }
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Char('a') | KeyCode::Char('A') => {
                            self.admin_mode_pending = false;
                            self.detail_popup = self.return_to_minion_detail.take()
                                .map(|mid| DetailPopup::Minion(mid, PingState::Idle, None, 0));
                        }
                        _ => {}
                    }
                }
                DetailPopup::SaltEvent { ref evt, scroll: ref mut scroll_offset, ref mut pretty, ref mut pretty_fetch } => {
                    // self.event_detail_scroll_max is the true viewport-aware
                    // last-scroll-position, recomputed and written back every
                    // render (see draw_event_detail) — clamping against it here
                    // (rather than against the popup's raw line count, or not at
                    // all) is what keeps Down/PageDown/End from running
                    // scroll_offset past where content actually ends, which
                    // previously made PageUp appear to do nothing for several
                    // presses after End.
                    match code {
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                            // Sniffer, if any, isn't stashed here — it lives in
                            // `self.sniffer` independent of `detail_popup` and
                            // reappears on its own once this popup closes.
                            if let Some((mid, sel)) = self.return_to_minion_events.take() {
                                self.detail_popup = Some(DetailPopup::MinionEvents {
                                    minion_id: mid, selected: sel,
                                });
                                self.popup_tab = Some(self.current_tab);
                            } else {
                                self.detail_popup = None;
                            }
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            *scroll_offset = scroll_offset.saturating_sub(1);
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            *scroll_offset = (*scroll_offset + 1).min(self.event_detail_scroll_max);
                        }
                        KeyCode::PageUp => {
                            *scroll_offset = scroll_offset.saturating_sub(10);
                        }
                        KeyCode::PageDown => {
                            *scroll_offset = (*scroll_offset + 10).min(self.event_detail_scroll_max);
                        }
                        KeyCode::Home => { *scroll_offset = 0; }
                        KeyCode::End => { *scroll_offset = self.event_detail_scroll_max; }
                        KeyCode::Char('y') => {
                            let ts = chrono::DateTime::from_timestamp(evt.ts as i64, 0)
                                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                                .unwrap_or_else(|| evt.ts.to_string());
                            clipboard_text = Some(format!(
                                "Tag: {}\nTimestamp: {}\n\nData:\n{}",
                                evt.tag,
                                ts,
                                event_body_as_copied_text(evt, *pretty, pretty_fetch),
                            ));
                        }
                        KeyCode::Char('Y') => {
                            clipboard_text = Some(event_body_as_copied_text(evt, *pretty, pretty_fetch));
                        }
                        KeyCode::Char('s') => {
                            let ts = chrono::DateTime::from_timestamp(evt.ts as i64, 0)
                                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                                .unwrap_or_else(|| evt.ts.to_string());
                            let text = format!(
                                "Tag: {}\nTimestamp: {}\n\nData:\n{}",
                                evt.tag,
                                ts,
                                event_body_as_copied_text(evt, *pretty, pretty_fetch),
                            );
                            save_text = Some((text, format!("event-{}", evt.tag), "txt"));
                        }
                        KeyCode::Char('S') => {
                            save_text = Some((
                                event_body_as_copied_text(evt, *pretty, pretty_fetch),
                                format!("event-{}", evt.tag),
                                if *pretty { "txt" } else { "json" },
                            ));
                        }
                        KeyCode::Char('t') | KeyCode::Char('T') => {
                            *pretty = !*pretty;
                            *scroll_offset = 0;
                            // Fetch the real `salt-run jobs.lookup_jid` text once
                            // per event (same subprocess + flags Job Detail uses
                            // -- see `cmd_jid_lookup` in the agent) instead of
                            // relying solely on the client-side approximation,
                            // so pretty-view colors/formatting match this
                            // master's actual Salt output. Only worth doing for
                            // a job-return event, and only once (Idle).
                            if *pretty && matches!(pretty_fetch, crate::types::PrettyFetch::Idle) {
                                let jid = evt.data.get("jid").and_then(|v| v.as_str());
                                let mid = evt.data.get("id").and_then(|v| v.as_str());
                                if let (Some(jid), Some(mid)) = (jid, mid) {
                                    trigger_event_pretty_lookup = Some((jid.to_string(), mid.to_string()));
                                    *pretty_fetch = crate::types::PrettyFetch::Loading;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                DetailPopup::Alert(ref alert) => {
                    match code {
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                            if let Some((mid, sel)) = self.return_to_minion_events.take() {
                                self.detail_popup = Some(DetailPopup::MinionEvents {
                                    minion_id: mid, selected: sel,
                                });
                            } else {
                                self.detail_popup = None;
                            }
                        }
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            let mut text = format!(
                                "Alert: {}\nSeverity: {}\nRule: {}\n{}\n",
                                alert.key,
                                alert.severity.label(),
                                alert.rule,
                                alert.description,
                            );
                            if !alert.details.is_empty() {
                                text.push_str("\nDetails:\n");
                                for d in &alert.details {
                                    text.push_str(&format!("  {}\n", d));
                                }
                            }
                            clipboard_text = Some(text);
                        }
                        KeyCode::Char('s') | KeyCode::Char('S') => {
                            let mut text = format!(
                                "Alert: {}\nSeverity: {}\nRule: {}\n{}\n",
                                alert.key,
                                alert.severity.label(),
                                alert.rule,
                                alert.description,
                            );
                            if !alert.details.is_empty() {
                                text.push_str("\nDetails:\n");
                                for d in &alert.details {
                                    text.push_str(&format!("  {}\n", d));
                                }
                            }
                            save_text = Some((text, format!("alert-{}", alert.key), "txt"));
                        }
                        _ => {}
                    }
                }
                DetailPopup::Minion(ref mid, _, ref mut grains, ref mut grains_scroll) => {
                    match code {
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                            self.detail_popup = None;
                        }
                        KeyCode::Char('g') | KeyCode::Char('G') if self.admin_mode => {
                            trigger_grains_fetch = Some(mid.clone());
                            *grains = Some(Vec::new());
                            *grains_scroll = 0;
                        }
                        KeyCode::Char('a') | KeyCode::Char('A') if self.admin_mode => {
                            self.admin_mode = false;
                        }
                        KeyCode::Char('a') | KeyCode::Char('A') if !self.admin_mode && !self.is_filtering => {
                            self.return_to_minion_detail = Some(mid.clone());
                            self.admin_mode_pending = true;
                            self.detail_popup = Some(DetailPopup::AdminConfirm);
                            self.popup_tab = Some(self.current_tab);
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            *grains_scroll = grains_scroll.saturating_sub(1);
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            let max = grains.as_ref().map_or(0, |p| p.len().saturating_sub(1));
                            *grains_scroll = (*grains_scroll + 1).min(max);
                        }
                        KeyCode::PageUp => {
                            *grains_scroll = grains_scroll.saturating_sub(10);
                        }
                        KeyCode::PageDown => {
                            let max = grains.as_ref().map_or(0, |p| p.len().saturating_sub(1));
                            *grains_scroll = (*grains_scroll + 10).min(max);
                        }
                        KeyCode::Home => { *grains_scroll = 0; }
                        KeyCode::End => {
                            *grains_scroll = grains.as_ref().map_or(0, |p| p.len().saturating_sub(1));
                        }
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            clipboard_text = Some(mid.clone());
                        }
                        KeyCode::Char('s') | KeyCode::Char('S') => {
                            save_text = Some((mid.clone(), format!("minion-{}", mid), "txt"));
                        }
                        _ => {}
                    }
                }
                DetailPopup::MinionEvents { ref minion_id, ref mut selected } => {
                    let items = minion_events_cache.as_ref()
                        .filter(|(id, _)| id == minion_id)
                        .map(|(_, e)| e.clone())
                        .unwrap_or_default();
                    let items_len = items.len();
                    let max_idx = items_len.saturating_sub(1);

                    if self.minion_events_is_filtering {
                        match code {
                            KeyCode::Esc => {
                                self.minion_events_is_filtering = false;
                                self.minion_events_filter.clear();
                            }
                            KeyCode::Enter => {
                                self.minion_events_is_filtering = false;
                                *selected = 0;
                            }
                            KeyCode::Backspace => {
                                self.minion_events_filter.pop();
                            }
                            KeyCode::Char(c) if !c.is_control() => {
                                self.minion_events_filter.push(c);
                            }
                            _ => {}
                        }
                    } else {
                        let evt_enter = match code {
                            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                                self.return_to_minion_events = None;
                                self.minion_events_is_filtering = false;
                                self.minion_events_filter.clear();
                                self.detail_popup = None;
                                None
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                *selected = selected.saturating_sub(1);
                                None
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                *selected = (*selected + 1).min(max_idx);
                                None
                            }
                            KeyCode::PageUp => {
                                *selected = selected.saturating_sub(10);
                                None
                            }
                            KeyCode::PageDown => {
                                *selected = (*selected + 10).min(max_idx);
                                None
                            }
                            KeyCode::Home => { *selected = 0; None }
                            KeyCode::End => { *selected = max_idx; None }
                            KeyCode::Char('/') => {
                                self.minion_events_is_filtering = true;
                                self.minion_events_filter.clear();
                                None
                            }
                            KeyCode::Enter => {
                                self.return_to_minion_events = Some((minion_id.clone(), *selected));
                                items.get(*selected).and_then(|item| match item {
                                    MinionActivityItem::Event(_, evt) => Some(evt.clone()),
                                    MinionActivityItem::CacheJob(job) => {
                                        let jid = job.jid.clone();
                                        let req_id = self.send_jid_lookup(&jid);
                                        self.pending_jid_lookup = Some((req_id, jid.clone()));
                                        self.pending_jid_lookup_at = Some(Instant::now());
                                        self.detail_popup = Some(DetailPopup::JobDetail {
                                            jid,
                                            lookup: None,
                                            selected_return: None,
                                            job_fun: job.fun.clone(),
                                            job_tgt: job.tgt.clone(),
                                            job_start_epoch: job.start_epoch,
                                            return_filter: String::new(),
                                            return_is_filtering: false,
                                            panel_focus: false,
                                            content_scroll: 0,
                                            content_scroll_mode: false,
                                        });
                                        self.popup_tab = Some(self.current_tab);
                                        None
                                    }
                                    MinionActivityItem::Alert(alert) => {
                                        self.detail_popup = Some(DetailPopup::Alert(alert.clone()));
                                        None
                                    }
                                })
                            }
                            KeyCode::Char('y') | KeyCode::Char('Y') => {
                                if let Some(item) = items.get(*selected) {
                                    match item {
                                        MinionActivityItem::Event(_, evt) => {
                                            clipboard_text = Some(serde_json::to_string_pretty(&evt.data)
                                                .unwrap_or_else(|_| evt.tag.clone()));
                                        }
                                        MinionActivityItem::CacheJob(job) => {
                                            clipboard_text = Some(format!("{} {} {} {}",
                                                job.jid, job.fun, job.tgt, job.start_time));
                                        }
                                        MinionActivityItem::Alert(alert) => {
                                            clipboard_text = Some(format!("[{}] {}: {}",
                                                alert.severity.label(), alert.rule, alert.description));
                                        }
                                    }
                                }
                                None
                            }
                            KeyCode::Char('s') | KeyCode::Char('S') => {
                                if let Some(item) = items.get(*selected) {
                                    match item {
                                        MinionActivityItem::Event(_, evt) => {
                                            let text = serde_json::to_string_pretty(&evt.data)
                                                .unwrap_or_else(|_| evt.tag.clone());
                                            save_text = Some((text, format!("event-{}", evt.tag), "txt"));
                                        }
                                        MinionActivityItem::CacheJob(job) => {
                                            let text = format!("{} {} {} {}",
                                                job.jid, job.fun, job.tgt, job.start_time);
                                            save_text = Some((text, format!("job-{}-{}", job.jid, minion_id), "txt"));
                                        }
                                        MinionActivityItem::Alert(alert) => {
                                            let text = format!("[{}] {}: {}",
                                                alert.severity.label(), alert.rule, alert.description);
                                            save_text = Some((text, format!("alert-{}", alert.key), "txt"));
                                        }
                                    }
                                }
                                None
                            }
                            _ => None,
                        };
                        if let Some(evt) = evt_enter {
                            self.detail_popup = Some(DetailPopup::SaltEvent { evt, scroll: 0, pretty: false, pretty_fetch: crate::types::PrettyFetch::Idle });
                        }
                    }
                }
                DetailPopup::ApiRequest(ref entry) => {
                    match code {
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                            self.detail_popup = None;
                        }
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            clipboard_text = Some(format!(
                                "{}\nClient: {}\nStatus: {}\nProcessing time: {:.3}s",
                                entry.raw_request_line, entry.client,
                                entry.response_status, entry.processing_time_secs,
                            ));
                        }
                        KeyCode::Char('s') | KeyCode::Char('S') => {
                            let text = format!(
                                "{}\nClient: {}\nStatus: {}\nProcessing time: {:.3}s",
                                entry.raw_request_line, entry.client,
                                entry.response_status, entry.processing_time_secs,
                            );
                            save_text = Some((text, format!("api-request-{}", entry.client), "txt"));
                        }
                        _ => {}
                    }
                }
                DetailPopup::ConfigEditor(_) => {}
            }
        }
        if let Some(text) = clipboard_text {
            self.copy_to_clipboard(&text);
            self.clipboard_at = Some(Instant::now());
            return false;
        }
        if let Some((content, hint, ext)) = save_text {
            self.open_save_prompt(content, &hint, ext);
            return false;
        }
        } else if sniffer_showing {
            if let Some(ref mut state) = self.sniffer {
                // Highlight keyword input mode
                if let Some(ref mut buf) = state.highlight_input {
                    match code {
                        KeyCode::Enter => {
                            let keywords: Vec<String> = buf.split(',')
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect();
                            if !keywords.is_empty() {
                                state.highlight_keywords = keywords;
                            }
                            state.highlight_input = None;
                        }
                        KeyCode::Esc => {
                            state.highlight_input = None;
                        }
                        KeyCode::Backspace => { buf.pop(); }
                        KeyCode::Char(c) => { buf.push(c); }
                        _ => {}
                    }
                    return false;
                }
                // Nav/selection/Enter/y all operate on the filtered
                // (displayed) view, not raw buffer indices — see
                // `SnifferState::filtered_indices`.
                let visible = state.filtered_indices();
                match code {
                    KeyCode::Enter => {
                        if let Some(evt) = visible.get(state.selected).and_then(|&i| state.buffer.get(i)) {
                            // `self.sniffer` isn't touched -- it keeps
                            // capturing underneath and reappears on its own
                            // once this SaltEvent popup closes.
                            self.detail_popup = Some(DetailPopup::SaltEvent { evt: evt.clone(), scroll: 0, pretty: false, pretty_fetch: crate::types::PrettyFetch::Idle });
                            self.popup_tab = Some(self.current_tab);
                        }
                    }
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                        self.sniffer = None;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        state.selected = state.selected.saturating_sub(1);
                        state.following = state.selected + 1 >= visible.len();
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        let max = visible.len().saturating_sub(1);
                        state.selected = state.selected.saturating_add(1).min(max);
                        state.following = state.selected + 1 >= visible.len();
                    }
                    KeyCode::PageUp => {
                        state.selected = state.selected.saturating_sub(10);
                        state.following = state.selected + 1 >= visible.len();
                    }
                    KeyCode::PageDown => {
                        let max = visible.len().saturating_sub(1);
                        state.selected = state.selected.saturating_add(10).min(max);
                        state.following = state.selected + 1 >= visible.len();
                    }
                    KeyCode::Home => {
                        state.selected = 0;
                        state.following = visible.len() <= 1;
                    }
                    KeyCode::End => {
                        state.selected = visible.len().saturating_sub(1);
                        state.following = true;
                    }
                    KeyCode::Char('p') | KeyCode::Char('P') => {
                        state.paused = !state.paused;
                    }
                    KeyCode::Char('h') | KeyCode::Char('H') => {
                        state.highlight_input = Some(state.highlight_keywords.join(", "));
                    }
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        let text = events_as_text(visible.iter().filter_map(|&i| state.buffer.get(i)));
                        if !text.is_empty() {
                            clipboard_text = Some(text);
                        }
                    }
                    KeyCode::Char('s') | KeyCode::Char('S') => {
                        let shown = visible.iter().filter_map(|&i| state.buffer.get(i));
                        let (text, ext) = if code == KeyCode::Char('S') {
                            (events_as_ndjson(shown), "json")
                        } else {
                            (events_as_text(shown), "txt")
                        };
                        if !text.is_empty() {
                            save_text = Some((text, format!("sniffer-{}", state.minion_ids.join("-")), ext));
                        }
                    }
                    _ => {}
                }
            }
            if let Some(text) = clipboard_text {
                self.copy_to_clipboard(&text);
                self.clipboard_at = Some(Instant::now());
                return false;
            }
            if let Some((content, hint, ext)) = save_text {
                self.open_save_prompt(content, &hint, ext);
                return false;
            }
        }
        if let Some((jid, mid)) = trigger_event_pretty_lookup {
            let req_id = self.send_jid_lookup(&jid);
            self.pending_event_pretty = Some((req_id, jid, mid));
            self.pending_event_pretty_at = Some(Instant::now());
        }
        if let Some(jid) = trigger_y_lookup {
            let req_id = self.send_jid_lookup(&jid);
            self.pending_y_copy = Some((req_id, jid));
        }
        if let Some(mid) = trigger_grains_fetch {
            self.send_agent_cmd("minion_grains", Some(serde_json::json!({"minion_id": mid})));
        }
        if had_popup || sniffer_showing {
            return false;
        }

        // Filter input mode (live jobs / other tabs)
        if self.is_filtering {
            match code {
                KeyCode::Esc => {
                    self.is_filtering = false;
                    self.filter_text.clear();
                    if self.current_tab == Tab::Events {
                        self.applied_filter.clear();
                        self.filtered_events_cache.clear();
                    } else {
                        self.commit_tab_filter("");
                    }
                    self.clamp_selection();
                    self.follow_events_tail_if_filtering();
                }
                KeyCode::Enter => {
                    self.is_filtering = false;
                    self.job_selected = 0;
                    self.alert_selected = 0;
                    self.minion_selected = 0;
                    if self.current_tab == Tab::Events {
                        // Committed here rather than kept live while typing --
                        // see the Backspace/Char arms below.
                        self.applied_filter = self.filter_text.clone();
                        self.rebuild_filtered_events_cache();
                        self.event_selected = self.filtered_events_len().saturating_sub(1);
                        self.events_following = true;
                    } else {
                        self.event_selected = 0;
                        let text = self.filter_text.clone();
                        self.commit_tab_filter(&text);
                    }
                    self.clamp_selection();
                }
                KeyCode::Backspace => {
                    self.filter_text.pop();
                    // Events applies its filter on Enter (see above), not per
                    // keystroke -- `self.applied_filter` doesn't change here,
                    // so re-clamping/re-following against it on every
                    // character is wasted work (and was the direct cause of
                    // typing itself stalling the TUI on masters with large
                    // event buffers). Other tabs still filter live.
                    if self.current_tab != Tab::Events {
                        self.clamp_active_filter_selection();
                        self.follow_events_tail_if_filtering();
                    }
                }
                KeyCode::Char(c) => {
                    self.filter_text.push(c);
                    if self.current_tab != Tab::Events {
                        self.clamp_active_filter_selection();
                        self.follow_events_tail_if_filtering();
                    }
                }
                _ => {}
            }
            return false;
        }

        // Cache filter input mode (Jobs tab, right panel)
        if self.cache_is_filtering {
            match code {
                KeyCode::Esc => {
                    self.cache_is_filtering = false;
                    self.cache_filter_text.clear();
                    self.cache_server_filter.clear();
                    self.cache_selected = 0;
                    self.clamp_selection();
                    self.send_cache_jobs_list(None, true);
                }
                KeyCode::Enter => {
                    self.cache_is_filtering = false;
                    self.cache_server_filter = self.cache_filter_text.clone();
                    self.cache_filter_text.clear();
                    self.cache_selected = 0;
                    self.clamp_selection();
                    let ft = self.cache_server_filter.clone();
                    self.send_cache_jobs_list(if ft.is_empty() { None } else { Some(&ft) }, true);
                }
                KeyCode::Backspace => {
                    self.cache_filter_text.pop();
                    self.clamp_active_filter_selection();
                }
                KeyCode::Char(c) => {
                    self.cache_filter_text.push(c);
                    self.clamp_active_filter_selection();
                }
                _ => {}
            }
            return false;
        }

        match code {
            KeyCode::Char('q') | KeyCode::Char('Q') => return true,
            KeyCode::Char('=') | KeyCode::Char('+') => {
                self.tick_interval_ms = (self.tick_interval_ms + 500).min(30_000);
            }
            KeyCode::Char('-') | KeyCode::Char('_') => {
                self.tick_interval_ms = self.tick_interval_ms.saturating_sub(500).max(500);
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                if self.admin_mode {
                    self.admin_mode = false;
                } else if self.detail_popup.is_none() && !self.is_filtering {
                    self.admin_mode_pending = true;
                    self.detail_popup = Some(DetailPopup::AdminConfirm);
                    self.popup_tab = Some(self.current_tab);
                }
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                if self.admin_mode && self.current_tab == Tab::Minions {
                    // Reset any stuck scan state and start fresh
                    self.fleet_scanning = true;
                    self.fleet_scan_completed = false;
                    self.fleet_scan_jid = None;
                    self.fleet_scan_started_at = None;
                    self.fleet_data = None;
                    self.send_agent_cmd("fleet_scan", None);
                } else if self.current_tab == Tab::Events && self.detail_popup.is_none() && self.sniffer.is_none() && !self.is_filtering {
                    let events = self.filtered_events();
                    if !events.is_empty() {
                        let (content, ext) = if code == KeyCode::Char('S') {
                            (events_as_ndjson(events.iter().copied()), "json")
                        } else {
                            (events_as_text(events.iter().copied()), "txt")
                        };
                        let hint = if self.applied_filter.is_empty() { "events".to_string() } else { format!("events-{}", self.applied_filter) };
                        self.open_save_prompt(content, &hint, ext);
                    }
                }
            }
            KeyCode::Char('x') | KeyCode::Char('X') => {
                if self.current_tab == Tab::Minions && self.fleet_data.is_some() {
                    self.hide_timed_out = !self.hide_timed_out;
                    self.clamp_selection();
                } else if self.current_tab == Tab::Api {
                    self.config.api_hide_stats_polls = !self.config.api_hide_stats_polls;
                }
            }
            KeyCode::Char('z') | KeyCode::Char('Z') => {
                if self.current_tab == Tab::Minions && !self.hide_timed_out {
                    self.timed_out_collapsed = !self.timed_out_collapsed;
                    self.clamp_selection();
                }
            }
            KeyCode::Char('f') | KeyCode::Char('F') => {
                if self.current_tab == Tab::Minions {
                    self.config.minions_follow_filter = !self.config.minions_follow_filter;
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                if self.current_tab == Tab::Events && self.detail_popup.is_none() && self.sniffer.is_none() && !self.is_filtering {
                    self.sniffer_input_active = true;
                    self.sniffer_input_text.clear();
                }
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                if self.current_tab == Tab::Minions {
                    if let Some(minion) = self.get_filtered_minion(self.minion_selected) {
                        self.detail_popup = Some(DetailPopup::MinionEvents {
                            minion_id: minion.id.clone(),
                            selected: 0,
                        });
                        self.popup_tab = Some(self.current_tab);
                        if !self.cache_loaded {
                            self.send_cache_jobs_list(None, false);
                        }
                    }
                } else if self.current_tab == Tab::Jobs && !self.cache_loading {
                    let ft = self.cache_server_filter.clone();
                    self.send_cache_jobs_list(if ft.is_empty() { None } else { Some(&ft) }, false);
                }
            }
            KeyCode::Char(']') | KeyCode::Char('}') => {
                if self.current_tab == Tab::Jobs && !self.cache_loading {
                    self.cache_max_results = (self.cache_max_results + 500).min(10000);
                    let ft = self.cache_server_filter.clone();
                    self.send_cache_jobs_list(if ft.is_empty() { None } else { Some(&ft) }, false);
                }
            }
            KeyCode::Char('[') | KeyCode::Char('{') => {
                if self.current_tab == Tab::Jobs && !self.cache_loading {
                    self.cache_max_results = self.cache_max_results.saturating_sub(500).max(200);
                    let ft = self.cache_server_filter.clone();
                    self.send_cache_jobs_list(if ft.is_empty() { None } else { Some(&ft) }, false);
                }
            }
            KeyCode::Char('>') | KeyCode::Char('.') => {
                if self.current_tab == Tab::Jobs && !self.cache_loading && self.cache_loaded {
                    self.cache_offset += self.cache_max_results;
                    if self.cache_offset >= self.cache_total_count {
                        self.cache_offset = self.cache_total_count.saturating_sub(self.cache_max_results);
                    }
                    let ft = self.cache_server_filter.clone();
                    self.send_cache_jobs_list(if ft.is_empty() { None } else { Some(&ft) }, false);
                }
            }
            KeyCode::Char('<') | KeyCode::Char(',') => {
                if self.current_tab == Tab::Jobs && !self.cache_loading && self.cache_loaded {
                    self.cache_offset = self.cache_offset.saturating_sub(self.cache_max_results);
                    let ft = self.cache_server_filter.clone();
                    self.send_cache_jobs_list(if ft.is_empty() { None } else { Some(&ft) }, false);
                }
            }
            KeyCode::Char('o') => {
                if let Some(table) = self.current_table() {
                    let next = Self::next_sort(self.sort_of(table), &self.columns(table));
                    self.set_sort(table, next);
                }
            }
            KeyCode::Char('O') => {
                if let Some(table) = self.current_table() {
                    if let Some((col, desc)) = self.sort_of(table) {
                        self.set_sort(table, Some((col, !desc)));
                    }
                }
            }
            KeyCode::Char('v') | KeyCode::Char('V') => {
                if let Some(table) = self.current_table() {
                    self.open_columns_picker(table);
                }
            }
            KeyCode::Char('?') | KeyCode::F(1) => {
                self.show_help = true;
                self.help_scroll = 0;
            }
            KeyCode::Char('m') | KeyCode::Char('M') => {
                self.sparkline_style = match self.sparkline_style {
                    SparklineStyle::Bars => SparklineStyle::Dots,
                    SparklineStyle::Dots => SparklineStyle::Bars,
                };
            }
            KeyCode::Char('/') => {
                if self.current_tab == Tab::Pulse {
                    // Pulse tab has no filter
                } else if self.current_tab == Tab::Jobs {
                    if self.cache_focus {
                        self.cache_is_filtering = true;
                        self.cache_filter_text.clear();
                        self.cache_server_filter.clear();
                    } else {
                        self.is_filtering = true;
                        self.filter_text.clear();
                    }
                } else {
                    self.is_filtering = true;
                    self.filter_text.clear();
                    if self.current_tab == Tab::Events {
                        self.applied_filter.clear();
                        self.filtered_events_cache.clear();
                    }
                }
            }
            KeyCode::Tab => {
                self.current_tab = self.current_tab.next();
            }
            KeyCode::BackTab => self.current_tab = self.current_tab.prev(),
            KeyCode::Left => {
                if self.current_tab == Tab::Jobs && !had_popup
                    && !self.is_filtering && !self.cache_is_filtering
                {
                    self.cache_focus = false;
                } else if self.current_tab == Tab::Pulse && !had_popup {
                    self.pulse_left();
                }
            }
            KeyCode::Right => {
                if self.current_tab == Tab::Jobs && !had_popup
                    && !self.is_filtering && !self.cache_is_filtering
                {
                    self.cache_focus = true;
                } else if self.current_tab == Tab::Pulse && !had_popup {
                    self.pulse_right();
                }
            }
            KeyCode::Char('1') => self.current_tab = Tab::Dashboard,
            KeyCode::Char('2') => self.current_tab = Tab::Minions,
            KeyCode::Char('3') => self.current_tab = Tab::Events,
            KeyCode::Char('4') => self.current_tab = Tab::Jobs,
            KeyCode::Char('5') => self.current_tab = Tab::Alerts,
            KeyCode::Char('6') => self.current_tab = Tab::Api,
            KeyCode::Char('7') => self.current_tab = Tab::Pulse,
            KeyCode::Up => {
                if self.current_tab == Tab::Pulse && !had_popup {
                    self.pulse_up();
                } else {
                    self.move_up();
                }
            }
            KeyCode::Down => {
                if self.current_tab == Tab::Pulse && !had_popup {
                    self.pulse_down();
                } else {
                    self.move_down();
                }
            }
            KeyCode::PageUp => { for _ in 0..10 { self.move_up(); } }
            KeyCode::PageDown => { for _ in 0..10 { self.move_down(); } }
            KeyCode::Home => self.move_to_first(),
            KeyCode::End => self.move_to_last(),
            KeyCode::Enter => self.open_detail(),
            KeyCode::F(2) => {
                self.mouse_enabled = !self.mouse_enabled;
                self.needs_mouse_toggle = true;
            }
            KeyCode::F(3) => self.open_config_editor(),
            KeyCode::F(4) if self.current_tab == Tab::Dashboard => {
                self.config.show_cpu_graph = !self.config.show_cpu_graph;
            }
            KeyCode::F(5) if self.current_tab == Tab::Dashboard => {
                self.config.show_mem_graph = !self.config.show_mem_graph;
            }
            KeyCode::F(6) if self.current_tab == Tab::Dashboard => {
                self.config.show_zmq_graph = !self.config.show_zmq_graph;
            }
            KeyCode::F(7) if self.current_tab == Tab::Dashboard => {
                self.config.show_events_graph = !self.config.show_events_graph;
            }
            KeyCode::F(8) if self.current_tab == Tab::Dashboard => {
                self.config.show_activity_graph = !self.config.show_activity_graph;
            }
            _ => {}
        }
        false
    }

    /// Handle a mouse click at the given (col, row) position.
    /// Row 1 = tab bar: switch to clicked tab.
    /// Content area = list tabs: select item by proportional position.
    pub fn handle_mouse_click(&mut self, col: u16, row: u16, frame_size: Rect) -> bool {
        let h = frame_size.height as usize;

        // Popup showing — AdminConfirm and MinionEvents handle left-click
        if self.detail_popup.is_some() && self.popup_tab == Some(self.current_tab) {
            if matches!(self.detail_popup, Some(DetailPopup::AdminConfirm)) {
                return self.handle_admin_confirm_click(col, row, frame_size);
            }
            if matches!(self.detail_popup, Some(DetailPopup::MinionEvents { .. })) {
                return self.handle_minion_events_click(col, row, frame_size);
            }
            // Other popups: ignore left-click (right-click closes from main.rs)
            return false;
        }

        // Tab bar click (row 1) — only within an actual tab label's column
        // range; clicking the blank space between/after labels does nothing
        // instead of silently falling through to Pulse (tab_at_col's old
        // catch-all made any row-1 click switch tabs, which made a single
        // stray/garbled mouse event — e.g. from a KVM switch's input
        // handoff — very likely to swap the view even though it didn't
        // land anywhere near a real tab).
        if row == 1 {
            if let Some(tab) = Self::tab_at_col(col) {
                self.current_tab = tab;
            }
            return false;
        }

        // Content area click (rows 3 to h-2; h-1 = key hints).
        // Layout (src/ui/mod.rs): title/tab frame is Length(3) (rows 0-2),
        // content area starts at row 3, status bar is the last row (h-1).
        let r = row as usize;
        if r < 3 || r >= h.saturating_sub(1) {
            return false;
        }

        if let Some(hit) = self.header_hits.iter().find(|h| h.y == row && col >= h.x0 && col < h.x1).copied() {
            match hit.table {
                TableId::LiveJobs => self.cache_focus = false,
                TableId::JobCache => self.cache_focus = true,
                _ => {}
            }
            self.set_sort(hit.table, Self::click_sort(self.sort_of(hit.table), hit.col));
            self.last_click_at = None;
            return false;
        }

        // Content area height = h-4 (title/tab frame + status bar).
        // Inner panel height = (h-4) - 2 (block top/bottom borders) = h-6.
        let inner_h = h.saturating_sub(6);

        match self.current_tab {
            Tab::Minions => {
                let total = self.filtered_minions_len();
                if total == 0 { return false; }

                // Must match draw_minion_list's own conditions exactly (src/ui/minions.rs)
                // -- unlike Events/Alerts/Jobs, Minions' own `visible` count is reduced
                // by this same overhead, not just its data_start.
                let filter_shown = self.is_filtering || !self.filter_text_for(Tab::Minions).is_empty() || self.hide_timed_out;
                let overhead = if filter_shown { 1 } else { 0 }
                    + if self.admin_mode { 1 } else { 0 };
                let visible = inner_h.saturating_sub(overhead).saturating_sub(2).max(1);

                let selected = self.minion_selected.min(total.saturating_sub(1));
                let scroll = scroll_offset(selected, visible, total);

                // Data starts at: inner.y(4) + overhead + header(1)
                let data_start = 4 + overhead + 1;
                if r >= data_start {
                    let visual_idx = r - data_start;
                    if visual_idx < visible {
                        self.minion_selected = (scroll + visual_idx).min(total - 1);
                        self.clamp_selection();
                    }
                }
            }
            Tab::Events => {
                // Events has 50/50 horizontal split: only left half is the event list
                let split_col = (frame_size.width / 2) as usize;
                if (col as usize) >= split_col {
                    return false;
                }
                let total = self.filtered_events_len();
                if total == 0 { return false; }

                // draw_event_list's `visible` always reserves 2 rows (filter+header)
                // regardless of whether the filter bar actually renders, but its
                // data_start only advances past the filter bar when it's actually
                // shown -- must match src/ui/events.rs exactly.
                let visible = inner_h.saturating_sub(2).max(1);
                let selected = self.event_selected.min(total.saturating_sub(1));
                let scroll = scroll_offset(selected, visible, total);

                let filter_shown = self.is_filtering || !self.applied_filter.is_empty();
                let data_start = 4 + if filter_shown { 1 } else { 0 } + 1;
                if r >= data_start {
                    let visual_idx = r - data_start;
                    if visual_idx < visible {
                        self.event_selected = (scroll + visual_idx).min(total - 1);
                        self.events_following = self.event_selected + 1 >= total;
                        self.clamp_selection();
                    }
                }
            }
            Tab::Alerts => {
                let total = self.filtered_alerts_len();
                if total == 0 { return false; }

                // Same shape as Events -- must match src/ui/alerts_view.rs exactly.
                let visible = inner_h.saturating_sub(2).max(1);
                let selected = self.alert_selected.min(total.saturating_sub(1));
                let scroll = scroll_offset(selected, visible, total);

                let filter_shown = self.is_filtering || !self.filter_text_for(Tab::Alerts).is_empty();
                let data_start = 4 + if filter_shown { 1 } else { 0 } + 1;
                if r >= data_start {
                    let visual_idx = r - data_start;
                    if visual_idx < visible {
                        self.alert_selected = (scroll + visual_idx).min(total - 1);
                        self.clamp_selection();
                    }
                }
            }
            Tab::Api => {
                let total = self.filtered_api_log_len();
                if total == 0 { return false; }

                // Same shape as Events -- must match draw_api's request-log panel exactly.
                let visible = inner_h.saturating_sub(2).max(1);
                let selected = self.api_log_selected.min(total.saturating_sub(1));
                let scroll = scroll_offset(selected, visible, total);

                let filter_shown = self.is_filtering || !self.filter_text_for(Tab::Api).is_empty();
                let data_start = 4 + if filter_shown { 1 } else { 0 } + 1;
                if r >= data_start {
                    let visual_idx = r - data_start;
                    if visual_idx < visible {
                        self.api_log_selected = (scroll + visual_idx).min(total - 1);
                        self.clamp_selection();
                    }
                }
            }
            Tab::Jobs => {
                // 55/45 split: left = live jobs, right = cache
                let col_split = frame_size.width as usize * 55 / 100;
                let live_click = (col as usize) < col_split;

                if self.cache_focus == live_click {
                    // Click is on the non-focused panel — just switch focus
                    self.cache_focus = !self.cache_focus;
                    return false;
                }

                if self.cache_focus {
                    let total = self.filtered_cache_len();
                    if total == 0 { return false; }

                    // Same shape as Events -- must match draw_cache_panel exactly.
                    let visible = inner_h.saturating_sub(2).max(1);
                    let selected = self.cache_selected.min(total.saturating_sub(1));
                    let scroll = scroll_offset(selected, visible, total);

                    let filter_shown = self.cache_is_filtering || !self.cache_server_filter.is_empty();
                    let data_start = 4 + if filter_shown { 1 } else { 0 } + 1;
                    if r >= data_start {
                        let visual_idx = r - data_start;
                        if visual_idx < visible {
                            self.cache_selected = (scroll + visual_idx).min(total - 1);
                            self.clamp_selection();
                        }
                    }
                } else {
                    let total = self.filtered_jobs_len();
                    if total == 0 { return false; }

                    // Same shape as Events -- must match draw_live_jobs_panel exactly.
                    let visible = inner_h.saturating_sub(2).max(1);
                    let selected = self.job_selected.min(total.saturating_sub(1));
                    let scroll = scroll_offset(selected, visible, total);

                    let filter_shown = self.is_filtering || !self.filter_text_for(Tab::Jobs).is_empty();
                    let data_start = 4 + if filter_shown { 1 } else { 0 } + 1;
                    if r >= data_start {
                        let visual_idx = r - data_start;
                        if visual_idx < visible {
                            self.job_selected = (scroll + visual_idx).min(total - 1);
                            self.jobs_following = self.job_selected + 1 >= total;
                            self.clamp_selection();
                        }
                    }
                }
            }
            Tab::Pulse => {
                // Hit-test against the dot grid's actual rendered area
                // (recorded by draw_dot_panel each frame) rather than
                // re-deriving its screen geometry here, so this can't
                // silently drift out of sync with the real layout again.
                let area = self.pulse_grid_area;
                let cx = col as usize;
                let cy = row as usize;

                if cy >= area.y as usize && cy < (area.y + area.height) as usize
                    && cx >= area.x as usize && cx < (area.x + area.width) as usize
                {
                    let grid_col = cx - area.x as usize;
                    let grid_row = cy - area.y as usize;
                    let cols = self.pulse_grid_cols.max(1);
                    let idx = grid_row * cols + grid_col;
                    let count = self.pulse_minions_count();
                    if idx < count {
                        self.pulse_selected = idx;
                    }
                }
            }
            Tab::Dashboard => {}
        }

        // Double-click detection on list tabs
        let dbl_click_ms: u128 = 400;
        if self.current_tab != Tab::Dashboard && self.current_tab != Tab::Pulse {
            let now = Instant::now();
            let is_double = self.last_click_at
                .map(|t| now.duration_since(t).as_millis() < dbl_click_ms && col == self.last_click_col && row == self.last_click_row)
                .unwrap_or(false);
            self.last_click_at = Some(now);
            self.last_click_col = col;
            self.last_click_row = row;
            if is_double {
                self.open_detail();
            }
        } else {
            self.last_click_at = None;
        }

        false
    }

    pub(crate) fn handle_admin_confirm_click(&mut self, col: u16, row: u16, frame_size: Rect) -> bool {
        let area = Self::centered_rect(55, 35, frame_size);
        let inside = row >= area.top() && row < area.top() + area.height
            && col >= area.left() && col < area.left() + area.width;

        if inside {
            let inner_x = col.saturating_sub(area.left() + 1);
            // "     [ OK ] Enter          [ Cancel ] Esc/q/a"
            //  [ OK ] at inner_x 5-11
            if (5..=11).contains(&inner_x) {
                self.admin_mode = true;
            }
            self.admin_mode_pending = false;
        } else {
            self.admin_mode_pending = false;
        }
        self.detail_popup = None;
        false
    }

    pub(crate) fn handle_minion_events_click(&mut self, col: u16, row: u16, frame_size: Rect) -> bool {
        let mid = match self.detail_popup {
            Some(DetailPopup::MinionEvents { ref minion_id, .. }) => minion_id.clone(),
            _ => return false,
        };

        let area = Self::centered_rect(85, 80, frame_size);
        let ix = area.x + 1;
        let iy = area.y + 1;
        let iw = area.width.saturating_sub(2);
        let ih = area.height.saturating_sub(2);

        if col < ix || col >= ix + iw
            || row < iy || row >= iy + ih.saturating_sub(1)
        {
            return false;
        }

        let para_row = (row - iy) as usize;
        if para_row < 4 {
            return false;
        }

        let items = self.minion_activity(&mid);
        let total = items.len();
        if total == 0 { return false; }

        let current_sel = match self.detail_popup {
            Some(DetailPopup::MinionEvents { selected, .. }) => selected,
            _ => return false,
        };
        let sel = current_sel.min(total.saturating_sub(1));

        let content_h = (ih as usize).saturating_sub(4);
        let scroll = if sel > content_h / 2 {
            sel.saturating_sub(content_h / 2).min(total.saturating_sub(1).saturating_sub(content_h / 2))
        } else {
            0
        };

        let item_idx = scroll + para_row - 4;
        if item_idx >= total { return false; }

        if let Some(DetailPopup::MinionEvents { ref mut selected, .. }) = self.detail_popup {
            *selected = item_idx;

            let dbl_click_ms: u128 = 400;
            let now = Instant::now();
            let is_double = self.last_click_at
                .map(|t| now.duration_since(t).as_millis() < dbl_click_ms && col == self.last_click_col && row == self.last_click_row)
                .unwrap_or(false);
            self.last_click_at = Some(now);
            self.last_click_col = col;
            self.last_click_row = row;
            if is_double {
                if let Some(item) = items.get(item_idx) {
                    match item {
                        MinionActivityItem::Event(_, evt) => {
                            self.return_to_minion_events = Some((mid, item_idx));
                            self.detail_popup = Some(DetailPopup::SaltEvent { evt: evt.clone(), scroll: 0, pretty: false, pretty_fetch: crate::types::PrettyFetch::Idle });
                        }
                        MinionActivityItem::CacheJob(job) => {
                            self.return_to_minion_events = Some((mid, item_idx));
                            let jid = job.jid.clone();
                            let req_id = self.send_jid_lookup(&jid);
                            self.pending_jid_lookup = Some((req_id, jid.clone()));
                            self.pending_jid_lookup_at = Some(Instant::now());
                            self.detail_popup = Some(DetailPopup::JobDetail {
                                jid,
                                lookup: None,
                                selected_return: None,
                                job_fun: job.fun.clone(),
                                job_tgt: job.tgt.clone(),
                                job_start_epoch: job.start_epoch,
                                return_filter: String::new(),
                                return_is_filtering: false,
                                panel_focus: false,
                                content_scroll: 0,
                                content_scroll_mode: false,
                            });
                            self.popup_tab = Some(self.current_tab);
                        }
                        MinionActivityItem::Alert(alert) => {
                            self.return_to_minion_events = Some((mid, item_idx));
                            self.detail_popup = Some(DetailPopup::Alert(alert.clone()));
                        }
                    }
                }
            }
        }

        false
    }

    pub(crate) fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
        let pad_v = (100u16.saturating_sub(percent_y)) / 2;
        let pad_h = (100u16.saturating_sub(percent_x)) / 2;
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(pad_v),
                Constraint::Percentage(percent_y),
                Constraint::Percentage(pad_v),
            ])
            .split(r);
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(pad_h),
                Constraint::Percentage(percent_x),
                Constraint::Percentage(pad_h),
            ])
            .split(vertical[1])[1]
    }

    pub(crate) fn move_up(&mut self) {
        match self.current_tab {
            Tab::Minions => self.minion_selected = self.minion_selected.saturating_sub(1),
            Tab::Events => {
                self.event_selected = self.event_selected.saturating_sub(1);
                self.events_following = self.event_selected + 1 >= self.filtered_events_len();
            }
            Tab::Jobs => {
                if self.cache_focus {
                    self.cache_selected = self.cache_selected.saturating_sub(1);
                } else {
                    self.job_selected = self.job_selected.saturating_sub(1);
                    self.jobs_following = self.job_selected + 1 >= self.filtered_jobs_len();
                }
            }
            Tab::Alerts => self.alert_selected = self.alert_selected.saturating_sub(1),
            Tab::Api => self.api_log_selected = self.api_log_selected.saturating_sub(1),
            Tab::Pulse => {
                let max = self.pulse_minions_count().saturating_sub(1);
                self.pulse_selected = self.pulse_selected.saturating_sub(1).min(max);
            }
            _ => {}
        }
    }

    pub(crate) fn move_down(&mut self) {
        match self.current_tab {
            Tab::Minions => self.minion_selected += 1,
            Tab::Events => {
                self.event_selected += 1;
                self.events_following = self.event_selected + 1 >= self.filtered_events_len();
            }
            Tab::Jobs => {
                if self.cache_focus {
                    self.cache_selected += 1;
                } else {
                    self.job_selected += 1;
                    self.jobs_following = self.job_selected + 1 >= self.filtered_jobs_len();
                }
            }
            Tab::Alerts => self.alert_selected += 1,
            Tab::Api => self.api_log_selected += 1,
            Tab::Pulse => {
                let max = self.pulse_minions_count().saturating_sub(1);
                self.pulse_selected = self.pulse_selected.saturating_add(1).min(max);
            }
            _ => {}
        }
    }

    pub(crate) fn move_to_first(&mut self) {
        match self.current_tab {
            Tab::Minions => self.minion_selected = 0,
            Tab::Events => {
                self.event_selected = 0;
                self.events_following = self.filtered_events_len() <= 1;
            }
            Tab::Jobs => {
                if self.cache_focus {
                    self.cache_selected = 0;
                } else {
                    self.job_selected = 0;
                    self.jobs_following = self.filtered_jobs_len() <= 1;
                }
            }
            Tab::Alerts => self.alert_selected = 0,
            Tab::Api => self.api_log_selected = 0,
            Tab::Pulse => self.pulse_selected = 0,
            _ => {}
        }
    }

    pub(crate) fn move_to_last(&mut self) {
        match self.current_tab {
            Tab::Minions => self.minion_selected = self.filtered_minions_len().saturating_sub(1),
            Tab::Events => {
                self.event_selected = self.filtered_events_len().saturating_sub(1);
                self.events_following = true;
            }
            Tab::Jobs => {
                if self.cache_focus {
                    self.cache_selected = self.filtered_cache_len().saturating_sub(1);
                } else {
                    self.job_selected = self.filtered_jobs_len().saturating_sub(1);
                    self.jobs_following = true;
                }
            }
            Tab::Alerts => self.alert_selected = self.filtered_alerts_len().saturating_sub(1),
            Tab::Api => self.api_log_selected = self.filtered_api_log_len().saturating_sub(1),
            Tab::Pulse => self.pulse_selected = self.pulse_minions_count().saturating_sub(1),
            _ => {}
        }
    }

    pub(crate) fn open_detail(&mut self) {
        match self.current_tab {
            Tab::Events => {
                self.return_to_minion_events = None;
                if let Some(evt) = self.get_filtered_event(self.event_selected) {
                    self.detail_popup = Some(DetailPopup::SaltEvent { evt: evt.clone(), scroll: 0, pretty: false, pretty_fetch: crate::types::PrettyFetch::Idle });
                    self.popup_tab = Some(self.current_tab);
                }
            }
            Tab::Jobs => {
                if self.cache_focus {
                    let cache_idx = self.cache_selected;
                    let (jid, fun, tgt, start_epoch, exists) = match self.get_filtered_cache_job(cache_idx) {
                        Some(job) => (job.jid.clone(), job.fun.clone(), job.tgt.clone(), job.start_epoch, true),
                        None => (String::new(), String::new(), String::new(), 0.0, false),
                    };
                    if exists {
                        let req_id = self.send_jid_lookup(&jid);
                        self.pending_jid_lookup = Some((req_id, jid.clone()));
                        self.pending_jid_lookup_at = Some(Instant::now());
                        self.detail_popup = Some(DetailPopup::JobDetail {
                            jid,
                            lookup: None,
                            selected_return: None,
                            job_fun: fun,
                            job_tgt: tgt,
                            job_start_epoch: start_epoch,
                            return_filter: String::new(),
                            return_is_filtering: false,
                            panel_focus: false,
                            content_scroll: 0,
                            content_scroll_mode: false,
                        });
                        self.popup_tab = Some(self.current_tab);
                    }
                } else {
                    let (jid, fun, tgt, start_epoch, exists) = {
                        let job = self.get_filtered_job(self.job_selected);
                        match job {
                            Some(job) => {
                                (job.jid.clone(), job.fun.clone(), job.tgt.clone(), job.created_epoch, true)
                            }
                            None => (String::new(), String::new(), String::new(), 0.0, false),
                        }
                    };
                    if exists {
                        let req_id = self.send_jid_lookup(&jid);
                        self.pending_jid_lookup = Some((req_id, jid.clone()));
                        self.pending_jid_lookup_at = Some(Instant::now());
                        self.detail_popup = Some(DetailPopup::JobDetail {
                            jid,
                            lookup: None,
                            selected_return: None,
                            job_fun: fun,
                            job_tgt: tgt,
                            job_start_epoch: start_epoch,
                            return_filter: String::new(),
                            return_is_filtering: false,
                            panel_focus: false,
                            content_scroll: 0,
                            content_scroll_mode: false,
                        });
                        self.popup_tab = Some(self.current_tab);
                    }
                }
            }
            Tab::Alerts => {
                if let Some(alert) = self.get_filtered_alert(self.alert_selected) {
                    self.detail_popup = Some(DetailPopup::Alert(alert.clone()));
                    self.popup_tab = Some(self.current_tab);
                }
            }
            Tab::Api => {
                if let Some(entry) = self.get_filtered_api_log(self.api_log_selected) {
                    self.detail_popup = Some(DetailPopup::ApiRequest(entry.clone()));
                    self.popup_tab = Some(self.current_tab);
                }
            }
            Tab::Minions => {
                if let Some(minion) = self.get_filtered_minion(self.minion_selected) {
                    let mid = minion.id.clone();
                    let state = if self.admin_mode {
                        self.send_agent_cmd("minion_ping", Some(serde_json::json!({"minion_id": &mid})));
                        PingState::Pinging(Instant::now())
                    } else {
                        PingState::Idle
                    };
                    self.detail_popup = Some(DetailPopup::Minion(mid, state, None, 0));
                    self.popup_tab = Some(self.current_tab);
                } else if !self.hide_timed_out {
                    // Header row selected — toggle collapse
                    self.timed_out_collapsed = !self.timed_out_collapsed;
                }
            }
            _ => {}
        }
    }
}
