//! Inert terminal comment list, separate from the source/annotation owners.
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Paragraph},
};
use vellum_model::{CommentLocationEvidence, CommentPlacement, CommentRow, CommentTarget};
pub fn draw_comments(
    frame: &mut Frame<'_>,
    rows: &[CommentRow],
    selected: usize,
    locked: bool,
    dirty: bool,
    notice: Option<&str>,
) {
    let [body, status, help] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let visible = usize::from(body.height.saturating_sub(2));
    let start = selected.saturating_sub(visible.saturating_sub(1));
    let lines: Vec<_> = rows
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .map(|(index, row)| {
            let location = match row.placement {
                CommentPlacement::Located { target, evidence } => {
                    let target = match target {
                        CommentTarget::Document => "document".into(),
                        CommentTarget::Point {
                            offset, profile, ..
                        } => format!("point byte {offset} {profile:?}"),
                        CommentTarget::ContentRange { start, end } => {
                            format!("bytes {start}..{end}")
                        }
                    };
                    format!(
                        "{target}{}",
                        if evidence == CommentLocationEvidence::Affected {
                            " affected"
                        } else {
                            ""
                        }
                    )
                }
                CommentPlacement::Detached => "detached".into(),
                CommentPlacement::Unavailable => "unavailable".into(),
            };
            let prefix = format!(
                "{} {} [{location}] ",
                if index == selected { ">" } else { " " },
                index + 1
            );
            let available = usize::from(body.width.saturating_sub(2)).saturating_sub(prefix.len());
            let preview: String = row
                .body
                .chars()
                .flat_map(char::escape_default)
                .take(available)
                .collect();
            Line::styled(
                format!("{prefix}{preview}"),
                if index == selected {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                },
            )
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(if dirty && locked {
            "Comments — UNSAVED [locked]"
        } else if dirty {
            "Comments — UNSAVED"
        } else if locked {
            "Comments — no unsaved changes [locked]"
        } else {
            "Comments — no unsaved changes"
        })),
        body,
    );
    frame.render_widget(
        Paragraph::new(notice.unwrap_or(
            "Source save does not persist comments; U/R may reverse linked native source history.",
        )),
        status,
    );
    frame.render_widget(
        Paragraph::new(
            "Enter edit | Del | A/P attach | U/R history | L lock | S/O JSON | D release failed read | Esc back",
        ),
        help,
    );
}
