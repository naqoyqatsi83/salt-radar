//! Keyboard and mouse input: the main key handler (tabs, lists and every
//! popup), cursor movement and click handling.

use super::*;

impl App {
    /// Opens help on the key reference, or on "What's new" right after an
    /// upgrade, which the title bar is pointing at.
    fn open_help(&mut self) {
        self.show_help = true;
        self.help_scroll = 0;
        self.help_page = if self.upgraded_from.take().is_some() { HelpPage::WhatsNew } else { HelpPage::Keys };
        self.help_search.clear();
        self.help_searching = false;
        self.help_search_focus = None;
    }

    /// Opens help on its Guide page, scrolled to the section titled `title`.
    fn open_help_at(&mut self, title: &str) {
        self.open_help();
        self.help_page = HelpPage::Guide;
        self.help_scroll = crate::ui::guide_section_line(title).unwrap_or(0) as u16;
    }

    fn handle_help_key(&mut self, code: KeyCode) {
        if self.help_searching {
            match code {
                KeyCode::Esc => {
                    self.help_searching = false;
                    self.help_search.clear();
                }
                KeyCode::Enter => {
                    self.help_searching = false;
                    self.help_search_jump = true;
                }
                _ => { line_edit::edit(&mut self.help_search, &mut self.input_cursor, code); }
            }
            return;
        }
        match code {
            KeyCode::Esc if !self.help_search.is_empty() => {
                self.help_search.clear();
                self.help_search_focus = None;
            }
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Char('?') | KeyCode::F(1) => {
                self.show_help = false;
            }
            KeyCode::Char('/') => {
                self.help_searching = true;
                self.help_search.clear();
                self.input_cursor = None;
                self.help_search_focus = None;
            }
            KeyCode::Char('n') | KeyCode::Char('N') => self.step_help_match(code == KeyCode::Char('n')),
            KeyCode::Tab | KeyCode::Right => self.show_help_page(self.help_page.step(true)),
            KeyCode::BackTab | KeyCode::Left => self.show_help_page(self.help_page.step(false)),
            KeyCode::Char('w') | KeyCode::Char('W') => self.show_help_page(HelpPage::WhatsNew),
            KeyCode::Up | KeyCode::Char('k') => self.help_scroll = self.help_scroll.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.help_scroll = self.help_scroll.saturating_add(1),
            KeyCode::PageUp => self.help_scroll = self.help_scroll.saturating_sub(10),
            KeyCode::PageDown => self.help_scroll = self.help_scroll.saturating_add(10),
            KeyCode::Home => self.help_scroll = 0,
            KeyCode::End => self.help_scroll = u16::MAX,
            _ => {}
        }
    }

    fn show_help_page(&mut self, page: HelpPage) {
        if self.help_page == page {
            return;
        }
        self.help_page = page;
        self.help_scroll = 0;
        self.help_search_focus = None;
        if page == HelpPage::WhatsNew {
            self.upgraded_from = None;
        }
    }

    /// Moves to the next/previous help search match and scrolls its line
    /// into view a couple of lines from the top (the draw clamps the scroll).
    fn step_help_match(&mut self, forward: bool) {
        let m = &self.help_search_matches;
        if let Some(i) = step_match(m, self.help_search_focus, self.help_scroll as usize, forward) {
            self.help_scroll = help_scroll_to(m[i]);
            self.help_search_focus = Some(i);
        }
    }

    pub fn handle_key(&mut self, code: KeyCode) -> bool {
        // Only an explicit yes quits: another `q` (lag, mashed keys) cancels.
        if self.quit_confirm {
            self.quit_confirm = false;
            return matches!(code, KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y'));
        }

        // Help overlay handles its own dismiss
        if self.show_help {
            self.handle_help_key(code);
            return false;
        }

        // Config editor is a global overlay — handle it early regardless of tab
        if self.config_editor.is_some() {
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
                    self.sniffer_input_error = None;
                }
                KeyCode::F(1) => self.open_help_at(crate::ui::SNIFFER_HELP_TITLE),
                KeyCode::Enter => {
                    match crate::sniff::parse_terms(&self.sniffer_input_text, |t| self.minions_for_term(t)) {
                        Err(e) => self.sniffer_input_error = Some(e),
                        Ok(terms) => {
                            if !terms.is_empty() {
                                let sniffer_max = self.config.sniffer_buffer_size;
                                self.sniffer = Some(crate::types::SnifferState {
                                    query: self.sniffer_input_text.trim().to_string(),
                                    terms,
                                    buffer: std::collections::VecDeque::with_capacity(sniffer_max),
                                    max_size: sniffer_max,
                                    paused: false,
                                    selected: 0,
                                    following: true,
                                    highlight_query: String::new(),
                                    highlight_terms: Vec::new(),
                                    highlight_input: None,
                                    highlight_error: None,
                                });
                            }
                            self.sniffer_input_active = false;
                            self.sniffer_input_text.clear();
                        }
                    }
                }
                _ => {
                    if line_edit::edit(&mut self.sniffer_input_text, &mut self.input_cursor, code) {
                        self.sniffer_input_error = None;
                    }
                }
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
                match &self.detail_popup {
                    Some(DetailPopup::JobDetail { return_is_filtering, .. }) => *return_is_filtering || self.job_searching,
                    Some(DetailPopup::SaltEvent { searching, .. }) | Some(DetailPopup::Alert { searching, .. }) => *searching,
                    _ => false,
                }
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
                    KeyCode::Char('7') => { self.current_tab = Tab::Master; return false; }
                    KeyCode::Char('8') => { self.current_tab = Tab::Pulse; return false; }
                    KeyCode::F(3) => { self.open_config_editor(); return false; }
                    KeyCode::F(2) => {
                        self.config.mouse = !self.config.mouse;
                        self.needs_mouse_toggle = true;
                        self.save_toggles();
                        return false;
                    }
                    KeyCode::Char('?') | KeyCode::F(1) => {
                        self.open_help();
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
            let items = self.minion_activity(minion_id);
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
                            KeyCode::Enter => { *return_is_filtering = false; }
                            _ => { line_edit::edit(return_filter, &mut self.input_cursor, code); }
                        }
                        if let Some(ref l) = lookup {
                            crate::app::snap_to_matching_return(l, selected_return, return_filter);
                        }
                        return false;
                    }
                    // Right panel (content preview) focused
                    let jump_ready = match code {
                        KeyCode::Char('n') | KeyCode::Char('N') => !self.job_search_matches.is_empty(),
                        KeyCode::Char('f') | KeyCode::Char('F') => !self.job_fail_lines.is_empty(),
                        _ => false,
                    };
                    if !*panel_focus && jump_ready && lookup.is_some() {
                        *panel_focus = true;
                        *content_scroll_mode = false;
                        *content_scroll = 0;
                    }
                    if *panel_focus {
                        if self.job_searching {
                            match code {
                                KeyCode::Esc => {
                                    self.job_searching = false;
                                    self.job_search.clear();
                                }
                                KeyCode::Enter => {
                                    self.job_searching = false;
                                    // Matched here, not taken from the last frame: keys typed
                                    // (or pasted) faster than a redraw would leave it stale.
                                    let shown = crate::ui::detail_popup::job_preview_lines(lookup.as_ref(), *selected_return);
                                    let m = search_lines(&crate::ui::detail_popup::plain_text(&shown), &self.job_search);
                                    if let Some(i) = step_match(&m, None, *content_scroll, true) {
                                        self.job_search_focus = Some(i);
                                        self.job_focus_line = None;
                                        *content_scroll = m[i].saturating_sub(2).min(self.job_detail_scroll_max);
                                    }
                                    self.job_search_matches = m;
                                }
                                _ => { line_edit::edit(&mut self.job_search, &mut self.input_cursor, code); }
                            }
                            return false;
                        }
                        match code {
                            KeyCode::Char('/') => {
                                self.job_searching = true;
                                self.job_search.clear();
                                self.input_cursor = None;
                                self.job_search_focus = None;
                                return false;
                            }
                            KeyCode::Char('n') | KeyCode::Char('N') if jump_ready => {
                                let m = &self.job_search_matches;
                                if let Some(i) = step_match(m, self.job_search_focus, *content_scroll, code == KeyCode::Char('n')) {
                                    *content_scroll = m[i].saturating_sub(2).min(self.job_detail_scroll_max);
                                    self.job_search_focus = Some(i);
                                    self.job_focus_line = None;
                                }
                                return false;
                            }
                            KeyCode::Char('f') | KeyCode::Char('F') if jump_ready => {
                                // A failure lands 4 lines down, on its state's `----------` separator.
                                let (m, context) = (&self.job_fail_lines, 4);
                                let cur = self.job_focus_line.filter(|l| m.contains(l));
                                let line = if code == KeyCode::Char('f') {
                                    match cur {
                                        Some(c) => m.iter().find(|&&l| l > c),
                                        None => m.iter().find(|&&l| l >= *content_scroll),
                                    }.or(m.first())
                                } else {
                                    match cur {
                                        Some(c) => m.iter().rev().find(|&&l| l < c),
                                        None => m.iter().rev().find(|&&l| l < *content_scroll),
                                    }.or(m.last())
                                }.copied();
                                if let Some(line) = line {
                                    self.job_focus_line = Some(line);
                                    self.job_search_focus = None;
                                    *content_scroll = line.saturating_sub(context).min(self.job_detail_scroll_max);
                                }
                                return false;
                            }
                            _ => {}
                        }
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
                    self.job_focus_line = None;
                    self.job_search_focus = None;
                    match code {
                        KeyCode::Esc if !return_filter.is_empty() => { return_filter.clear(); }
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
                            self.input_cursor = None;
                        }
                        KeyCode::Up | KeyCode::Char('k') | KeyCode::Down | KeyCode::Char('j')
                        | KeyCode::PageUp | KeyCode::PageDown | KeyCode::Home | KeyCode::End => {
                            if let Some(ref l) = lookup {
                                let visible: Vec<usize> = (0..l.entry_count())
                                    .filter(|&i| l.entry_id(i).is_some_and(|id| crate::app::return_matches(id, return_filter)))
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
                DetailPopup::SaltEvent { ref evt, scroll: ref mut scroll_offset, ref mut pretty, ref mut pretty_fetch, ref mut search, ref mut searching, ref mut search_focus } => {
                    if *searching {
                        match code {
                            KeyCode::Esc => {
                                *searching = false;
                                search.clear();
                            }
                            KeyCode::Enter => {
                                *searching = false;
                                // Matched here, not taken from the last frame: keys typed
                                // (or pasted) faster than a redraw would leave it stale.
                                let m = search_lines(&event_body_as_shown(evt, *pretty, pretty_fetch), search);
                                if let Some(i) = step_match(&m, None, *scroll_offset, true) {
                                    *search_focus = Some(i);
                                    *scroll_offset = m[i].saturating_sub(2).min(self.event_detail_scroll_max);
                                }
                                self.event_search_matches = m;
                            }
                            _ => { line_edit::edit(search, &mut self.input_cursor, code); }
                        }
                        return false;
                    }
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
                        KeyCode::Char('/') => {
                            *searching = true;
                            search.clear();
                            self.input_cursor = None;
                            *search_focus = None;
                        }
                        KeyCode::Char('n') | KeyCode::Char('N') => {
                            let m = &self.event_search_matches;
                            if let Some(i) = step_match(m, *search_focus, *scroll_offset, code == KeyCode::Char('n')) {
                                *scroll_offset = m[i].saturating_sub(2).min(self.event_detail_scroll_max);
                                *search_focus = Some(i);
                            }
                        }
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
                            *search_focus = None;
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
                DetailPopup::Alert { ref alert, ref mut scroll, ref mut search, ref mut searching, ref mut search_focus } => {
                    if *searching {
                        match code {
                            KeyCode::Esc => {
                                *searching = false;
                                search.clear();
                            }
                            KeyCode::Enter => {
                                *searching = false;
                                let m = search_lines(&alert.details.join("\n"), search);
                                if let Some(i) = step_match(&m, None, *scroll, true) {
                                    *search_focus = Some(i);
                                    *scroll = m[i].saturating_sub(2).min(self.alert_detail_scroll_max);
                                }
                                self.alert_search_matches = m;
                            }
                            _ => { line_edit::edit(search, &mut self.input_cursor, code); }
                        }
                        return false;
                    }
                    let max = self.alert_detail_scroll_max;
                    match code {
                        KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
                        KeyCode::Down | KeyCode::Char('j') => *scroll = (*scroll + 1).min(max),
                        KeyCode::PageUp => *scroll = scroll.saturating_sub(10),
                        KeyCode::PageDown => *scroll = (*scroll + 10).min(max),
                        KeyCode::Home => *scroll = 0,
                        KeyCode::End => *scroll = max,
                        KeyCode::Char('/') => {
                            *searching = true;
                            search.clear();
                            self.input_cursor = None;
                            *search_focus = None;
                        }
                        KeyCode::Char('n') | KeyCode::Char('N') => {
                            let m = &self.alert_search_matches;
                            if let Some(i) = step_match(m, *search_focus, *scroll, code == KeyCode::Char('n')) {
                                *scroll = m[i].saturating_sub(2).min(max);
                                *search_focus = Some(i);
                            }
                        }
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
                            _ => { line_edit::edit(&mut self.minion_events_filter, &mut self.input_cursor, code); }
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
                                self.input_cursor = None;
                                None
                            }
                            KeyCode::Enter => {
                                self.return_to_minion_events = Some((minion_id.clone(), *selected));
                                let mid = minion_id.clone();
                                items.get(*selected).and_then(|item| match item {
                                    MinionActivityItem::Event(_, evt) => match self.return_event_job(evt) {
                                        Some((jid, header)) => {
                                            self.open_job_detail(jid, header, Some(mid));
                                            None
                                        }
                                        None => Some(evt.clone()),
                                    },
                                    MinionActivityItem::CacheJob(job) => {
                                        self.open_job_detail(job.jid.clone(), cache_job_header(job), Some(mid));
                                        None
                                    }
                                    MinionActivityItem::Alert(alert) => {
                                        self.detail_popup = Some(DetailPopup::alert(alert.clone()));
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
                            self.detail_popup = Some(DetailPopup::event(evt));
                        }
                    }
                }
                DetailPopup::MasterSetting { index, ref mut scroll } => {
                    let max = self.master_popup_scroll_max;
                    let setting = self.master_config.as_ref().and_then(|mc| mc.settings.get(index));
                    match code {
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                            self.detail_popup = None;
                        }
                        KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
                        KeyCode::Down | KeyCode::Char('j') => *scroll = (*scroll + 1).min(max),
                        KeyCode::PageUp => *scroll = scroll.saturating_sub(10),
                        KeyCode::PageDown => *scroll = (*scroll + 10).min(max),
                        KeyCode::Home => *scroll = 0,
                        KeyCode::End => *scroll = max,
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            clipboard_text = setting.map(ConfigSetting::as_yaml);
                        }
                        KeyCode::Char('s') | KeyCode::Char('S') => {
                            save_text = setting.map(|s| (s.as_yaml(), format!("master-{}", s.key), "txt"));
                        }
                        _ => {}
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
                        KeyCode::Enter => match crate::sniff::parse_terms(buf, |_| Vec::new()) {
                            Ok(terms) => {
                                state.highlight_query = buf.trim().to_string();
                                state.highlight_terms = terms;
                                state.highlight_input = None;
                                state.highlight_error = None;
                            }
                            Err(e) => state.highlight_error = Some(e),
                        },
                        KeyCode::Esc => {
                            state.highlight_input = None;
                            state.highlight_error = None;
                        }
                        KeyCode::F(1) => {
                            self.open_help_at(crate::ui::SNIFFER_HELP_TITLE);
                            return false;
                        }
                        _ => {
                            if line_edit::edit(buf, &mut self.input_cursor, code) {
                                state.highlight_error = None;
                            }
                        }
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
                            self.detail_popup = Some(DetailPopup::event(evt.clone()));
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
                        state.highlight_input = Some(state.highlight_query.clone());
                        self.input_cursor = None;
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
                            save_text = Some((text, format!("sniffer-{}", state.query), ext));
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
                _ => {
                    // Events applies its filter on Enter (see above), not per
                    // keystroke -- `self.applied_filter` doesn't change here,
                    // so re-clamping/re-following against it on every
                    // character is wasted work (and was the direct cause of
                    // typing itself stalling the TUI on masters with large
                    // event buffers). Other tabs still filter live.
                    if line_edit::edit(&mut self.filter_text, &mut self.input_cursor, code) && self.current_tab != Tab::Events {
                        self.clamp_active_filter_selection();
                        self.follow_events_tail_if_filtering();
                    }
                }
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
                _ => {
                    if line_edit::edit(&mut self.cache_filter_text, &mut self.input_cursor, code) {
                        self.clamp_active_filter_selection();
                    }
                }
            }
            return false;
        }

        match code {
            KeyCode::Char('q') | KeyCode::Char('Q') => {
                if !self.config.confirm_quit {
                    return true;
                }
                self.quit_confirm = true;
            }
            KeyCode::Char('=') | KeyCode::Char('+') => {
                self.config.tick_ms = (self.config.tick_ms + 500).min(crate::config::TICK_MS_MAX);
                self.save_toggles();
            }
            KeyCode::Char('-') | KeyCode::Char('_') => {
                self.config.tick_ms = self.config.tick_ms.saturating_sub(500).max(crate::config::TICK_MS_MIN);
                self.save_toggles();
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
                    self.save_toggles();
                } else if self.current_tab == Tab::Jobs && !self.cache_focus {
                    self.jobs_failed_only = !self.jobs_failed_only;
                    self.job_selected = 0;
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
                    self.save_toggles();
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                if self.current_tab == Tab::Events && self.detail_popup.is_none() && self.sniffer.is_none() && !self.is_filtering {
                    self.sniffer_input_active = true;
                    self.sniffer_input_text.clear();
                    self.input_cursor = None;
                    self.sniffer_input_error = None;
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
                } else if self.current_tab == Tab::Master && !self.master_config_loading {
                    self.request_master_config();
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
                self.open_help();
            }
            KeyCode::Char('m') | KeyCode::Char('M') => {
                self.config.graph_style = match self.config.sparkline_style() {
                    SparklineStyle::Bars => "dots",
                    SparklineStyle::Dots => "bars",
                }.to_string();
                self.save_toggles();
            }
            KeyCode::Char('/') => {
                if matches!(self.current_tab, Tab::Dashboard | Tab::Pulse) {
                    // Nothing to filter on these tabs
                } else if self.current_tab == Tab::Jobs {
                    if self.cache_focus {
                        self.cache_is_filtering = true;
                        self.cache_filter_text.clear();
                        self.input_cursor = None;
                        self.cache_server_filter.clear();
                    } else {
                        self.is_filtering = true;
                        self.filter_text.clear();
                        self.input_cursor = None;
                    }
                } else {
                    self.is_filtering = true;
                    self.filter_text.clear();
                    self.input_cursor = None;
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
            KeyCode::Char('7') => self.current_tab = Tab::Master,
            KeyCode::Char('8') => self.current_tab = Tab::Pulse,
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
                self.config.mouse = !self.config.mouse;
                self.needs_mouse_toggle = true;
                self.save_toggles();
            }
            KeyCode::F(3) => self.open_config_editor(),
            KeyCode::F(4) if self.current_tab == Tab::Dashboard => {
                self.config.show_cpu_graph = !self.config.show_cpu_graph;
                self.save_toggles();
            }
            KeyCode::F(5) if self.current_tab == Tab::Dashboard => {
                self.config.show_mem_graph = !self.config.show_mem_graph;
                self.save_toggles();
            }
            KeyCode::F(6) if self.current_tab == Tab::Dashboard => {
                self.config.show_zmq_graph = !self.config.show_zmq_graph;
                self.save_toggles();
            }
            KeyCode::F(7) if self.current_tab == Tab::Dashboard => {
                self.config.show_events_graph = !self.config.show_events_graph;
                self.save_toggles();
            }
            KeyCode::F(8) if self.current_tab == Tab::Dashboard => {
                self.config.show_activity_graph = !self.config.show_activity_graph;
                self.save_toggles();
            }
            _ => {}
        }
        false
    }

    /// Handle a mouse click at the given (col, row) position.
    /// Row 1 = tab bar: switch to clicked tab.
    /// Content area = list tabs: select item by proportional position.
    /// `Ctrl+←`/`→`: moves the current tab's divider between its two panels
    /// by 1%. False when there's no divider to move (no split on this tab,
    /// or an overlay or popup in front), so the key acts as a plain arrow.
    pub fn move_split(&mut self, right: bool) -> bool {
        let popup_here = self.detail_popup.is_some() && self.popup_tab == Some(self.current_tab);
        if self.quit_confirm || self.show_help || self.config_editor.is_some() || self.save_prompt.is_some() || popup_here {
            return false;
        }
        let Some(pct) = self.config.split_pct_mut(self.current_tab) else { return false };
        let moved = if right { *pct + 1 } else { pct.saturating_sub(1) };
        *pct = moved.clamp(crate::config::SPLIT_PCT_MIN, crate::config::SPLIT_PCT_MAX);
        self.save_toggles();
        true
    }

    /// The column where the current tab's right panel starts.
    fn split_col(&self, frame_size: Rect) -> usize {
        crate::ui::split_panes(frame_size, self)[1].x as usize
    }

    pub fn handle_mouse_click(&mut self, col: u16, row: u16, frame_size: Rect) -> bool {
        let h = frame_size.height as usize;

        if self.quit_confirm {
            self.quit_confirm = false;
            return false;
        }
        if self.config_editor.is_some() || self.save_prompt.is_some() {
            return false;
        }
        if self.show_help {
            if let Some(&(_, _, _, page)) = self.help_page_hits.iter().find(|&&(y, x0, x1, _)| y == row && (x0..x1).contains(&col)) {
                if !self.help_searching {
                    self.show_help_page(page);
                }
            }
            return false;
        }

        // Popup showing — AdminConfirm and MinionEvents handle left-click
        if self.detail_popup.is_some() && self.popup_tab == Some(self.current_tab) {
            if matches!(self.detail_popup, Some(DetailPopup::AdminConfirm)) {
                return self.handle_admin_confirm_click(col, row, frame_size);
            }
            if matches!(self.detail_popup, Some(DetailPopup::MinionEvents { .. })) {
                return self.handle_minion_events_click(col, row, frame_size);
            }
            if matches!(self.detail_popup, Some(DetailPopup::JobDetail { .. })) {
                return self.handle_job_detail_click(col, row);
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

        // The sniffer overlays the Events list: a click there would open an
        // event from the list behind it.
        if self.current_tab == Tab::Events && self.sniffer.is_some() {
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
                // Only the left panel is the event list
                if (col as usize) >= self.split_col(frame_size) {
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
            Tab::Master => {
                if (col as usize) >= self.split_col(frame_size) {
                    return false;
                }
                let total = self.filtered_master_settings().len();
                if total == 0 { return false; }

                // Same shape as Events -- must match draw_master_list exactly.
                let visible = inner_h.saturating_sub(2).max(1);
                let selected = self.master_selected.min(total.saturating_sub(1));
                let scroll = scroll_offset(selected, visible, total);

                let filter_shown = self.is_filtering || !self.filter_text_for(Tab::Master).is_empty();
                let data_start = 4 + if filter_shown { 1 } else { 0 } + 1;
                if r >= data_start {
                    let visual_idx = r - data_start;
                    if visual_idx < visible {
                        self.master_selected = (scroll + visual_idx).min(total - 1);
                    }
                }
            }
            Tab::Jobs => {
                let live_click = (col as usize) < self.split_col(frame_size);

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

        if self.current_tab != Tab::Dashboard && self.current_tab != Tab::Pulse {
            if self.register_click(col, row) {
                self.open_detail();
            }
        } else {
            self.last_click_at = None;
        }

        false
    }

    /// Records a click and tells whether it completes a double-click: a
    /// second click on the same cell within 400ms.
    fn register_click(&mut self, col: u16, row: u16) -> bool {
        let now = Instant::now();
        let is_double = self.last_click_at
            .is_some_and(|t| now.duration_since(t).as_millis() < 400 && col == self.last_click_col && row == self.last_click_row);
        self.last_click_at = Some(now);
        self.last_click_col = col;
        self.last_click_row = row;
        is_double
    }

    /// Job Detail (#164): a click on a returns row selects that minion, a
    /// double-click opens its return (like Enter), and a click on the
    /// preview focuses it.
    fn handle_job_detail_click(&mut self, col: u16, row: u16) -> bool {
        let inside = |r: Rect| r.contains(ratatui::layout::Position { x: col, y: row });
        let hits = &self.job_detail_hits;
        let clicked = hits.rows.iter().find(|&&(y, _)| y == row).map(|&(_, i)| i).filter(|_| inside(hits.list));
        let on_preview = inside(hits.preview);
        let is_double = clicked.is_some() && self.register_click(col, row);
        if self.job_searching {
            return false;
        }
        let Some(DetailPopup::JobDetail {
            ref mut selected_return, ref mut panel_focus, ref mut content_scroll,
            ref mut content_scroll_mode, ref lookup, return_is_filtering, ..
        }) = self.detail_popup else { return false };
        if return_is_filtering || lookup.is_none() {
            return false;
        }
        if let Some(i) = clicked {
            if *selected_return != Some(i) {
                *content_scroll = 0;
                self.job_focus_line = None;
                self.job_search_focus = None;
            }
            *selected_return = Some(i);
            *panel_focus = is_double;
            *content_scroll_mode = false;
        } else if on_preview {
            *panel_focus = true;
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

        let header = MINION_EVENTS_HEADER_ROWS as usize;
        let para_row = (row - iy) as usize;
        if para_row < header {
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
        let list_h = (ih as usize).saturating_sub(header + self.minion_events_is_filtering as usize);
        let (scroll, _, _) = crate::ui::scroll_window(sel, list_h, total);

        let item_idx = scroll + para_row - header;
        if item_idx >= total { return false; }

        if let Some(DetailPopup::MinionEvents { ref mut selected, .. }) = self.detail_popup {
            *selected = item_idx;
            if self.register_click(col, row) {
                if let Some(item) = items.get(item_idx) {
                    match item {
                        MinionActivityItem::Event(_, evt) => {
                            self.return_to_minion_events = Some((mid.clone(), item_idx));
                            match self.return_event_job(evt) {
                                Some((jid, header)) => self.open_job_detail(jid, header, Some(mid)),
                                None => self.detail_popup = Some(DetailPopup::event(evt.clone())),
                            }
                        }
                        MinionActivityItem::CacheJob(job) => {
                            self.return_to_minion_events = Some((mid.clone(), item_idx));
                            self.open_job_detail(job.jid.clone(), cache_job_header(job), Some(mid));
                        }
                        MinionActivityItem::Alert(alert) => {
                            self.return_to_minion_events = Some((mid, item_idx));
                            self.detail_popup = Some(DetailPopup::alert(alert.clone()));
                        }
                    }
                }
            }
        }

        false
    }

    /// Opens Job Detail and starts its lookup; `select_minion` picks that
    /// minion's return once the data arrives.
    pub(crate) fn open_job_detail(&mut self, jid: String, job: JobHeader, select_minion: Option<String>) {
        let req_id = self.send_jid_lookup(&jid);
        self.pending_jid_lookup = Some((req_id, jid.clone()));
        self.pending_jid_lookup_at = Some(Instant::now());
        self.job_detail_select_minion = select_minion;
        self.job_search.clear();
        self.job_searching = false;
        self.job_search_matches.clear();
        self.job_search_focus = None;
        self.job_fail_lines.clear();
        self.job_focus_line = None;
        self.detail_popup = Some(DetailPopup::JobDetail {
            jid,
            lookup: None,
            selected_return: None,
            job,
            return_filter: String::new(),
            return_is_filtering: false,
            panel_focus: false,
            content_scroll: 0,
            content_scroll_mode: false,
        });
        self.popup_tab = Some(self.current_tab);
    }

    /// The job behind a return event. Return events carry no target, so the
    /// header comes from Live Jobs when the job is there.
    fn return_event_job(&self, evt: &SaltEvent) -> Option<(String, JobHeader)> {
        let jid = return_event_jid(evt)?.to_string();
        if let Some(live) = self.jobs.iter().find(|j| j.jid == jid) {
            return Some((jid, live_job_header(live)));
        }
        let fun = evt.data["fun"].as_str().unwrap_or("?").to_string();
        let (args, test_run) = format_job_args(&fun, &evt.data["fun_args"]);
        let tgt = evt.data["id"].as_str().unwrap_or("").to_string();
        Some((jid, JobHeader { fun, tgt, args, test_run, start_epoch: evt.ts }))
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
            Tab::Master => self.master_selected = self.master_selected.saturating_sub(1),
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
            Tab::Master => {
                let max = self.filtered_master_settings().len().saturating_sub(1);
                self.master_selected = (self.master_selected + 1).min(max);
            }
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
            Tab::Master => self.master_selected = 0,
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
            Tab::Master => self.master_selected = self.filtered_master_settings().len().saturating_sub(1),
            Tab::Pulse => self.pulse_selected = self.pulse_minions_count().saturating_sub(1),
            _ => {}
        }
    }

    pub(crate) fn open_detail(&mut self) {
        match self.current_tab {
            Tab::Events => {
                self.return_to_minion_events = None;
                if let Some(evt) = self.get_filtered_event(self.event_selected) {
                    self.detail_popup = Some(DetailPopup::event(evt.clone()));
                    self.popup_tab = Some(self.current_tab);
                }
            }
            Tab::Jobs => {
                if self.cache_focus {
                    let cache_idx = self.cache_selected;
                    if let Some(job) = self.get_filtered_cache_job(cache_idx) {
                        let header = JobHeader {
                            fun: job.fun.clone(), tgt: job.tgt.clone(), args: job.args.clone(),
                            test_run: job.test_run, start_epoch: job.start_epoch,
                        };
                        self.open_job_detail(job.jid.clone(), header, None);
                    }
                } else {
                    if let Some(job) = self.get_filtered_job(self.job_selected) {
                        let jid = job.jid.clone();
                        let header = live_job_header(job);
                        self.open_job_detail(jid, header, None);
                    }
                }
            }
            Tab::Alerts => {
                if let Some(alert) = self.get_filtered_alert(self.alert_selected) {
                    self.detail_popup = Some(DetailPopup::alert(alert.clone()));
                    self.popup_tab = Some(self.current_tab);
                }
            }
            Tab::Api => {
                if let Some(entry) = self.get_filtered_api_log(self.api_log_selected) {
                    self.detail_popup = Some(DetailPopup::ApiRequest(entry.clone()));
                    self.popup_tab = Some(self.current_tab);
                }
            }
            Tab::Master => {
                if let Some(&index) = self.filtered_master_settings().get(self.master_selected) {
                    self.detail_popup = Some(DetailPopup::MasterSetting { index, scroll: 0 });
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

fn live_job_header(job: &JobEntry) -> JobHeader {
    JobHeader {
        fun: job.fun.clone(), tgt: job.tgt.clone(), args: job.args.clone(),
        test_run: job.test_run, start_epoch: job.created_epoch,
    }
}

fn cache_job_header(job: &CacheJobEntry) -> JobHeader {
    JobHeader {
        fun: job.fun.clone(), tgt: job.tgt.clone(), args: job.args.clone(),
        test_run: job.test_run, start_epoch: job.start_epoch,
    }
}
