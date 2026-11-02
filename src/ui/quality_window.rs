//! Vérifications qualité (D-05): the state of the window; the drawing and
//! the operations live in `app.rs`.

use std::collections::BTreeSet;

use crate::quality::Report;
use crate::replace::Scope;

/// What the worker sends back.
pub enum Msg {
    Progress(usize, usize),
    Done(Report),
}

pub struct Job {
    pub rx: std::sync::mpsc::Receiver<Msg>,
    pub progress: f32,
    /// The document the run is for.
    pub serial: u64,
}

#[derive(Default)]
pub struct QualityWindow {
    pub open: bool,
    pub needs_place: bool,
    pub scope: Scope,
    pub job: Option<Job>,
    pub report: Option<Report>,
    /// Rows of the table: finding indices in the sorted order.
    pub rows: Vec<usize>,
    /// Selected rows (row indices).
    pub selected: BTreeSet<usize>,
    /// Column and direction of the sort.
    pub sort: (usize, bool),
    pub message: Option<String>,
}

impl QualityWindow {
    /// Rebuild `rows` from the report in the sort order (line, EDID,
    /// check, detail); the EDID and detail sorts use the line as a proxy
    /// and the check order respectively — stable enough for a results list.
    pub fn sort_rows(&mut self) {
        let Some(report) = &self.report else {
            self.rows.clear();
            return;
        };
        let mut rows: Vec<usize> = (0..report.findings.len()).collect();
        let (col, asc) = self.sort;
        rows.sort_by(|&a, &b| {
            let (fa, fb) = (&report.findings[a], &report.findings[b]);
            let ord = match col {
                2 => fa.check.cmp(&fb.check).then(fa.line.cmp(&fb.line)),
                3 => format!("{:?}", fa.detail).cmp(&format!("{:?}", fb.detail)),
                _ => fa.line.cmp(&fb.line).then(fa.check.cmp(&fb.check)),
            };
            if asc { ord } else { ord.reverse() }
        });
        self.rows = rows;
        self.selected.retain(|r| *r < self.rows.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quality::{Check, Detail, Finding};

    #[test]
    fn rows_follow_the_sort() {
        let mut w = QualityWindow {
            report: Some(Report {
                findings: vec![
                    Finding {
                        line: 5,
                        check: Check::Spaces,
                        detail: Detail::DoubleSpace,
                    },
                    Finding {
                        line: 2,
                        check: Check::Aliases,
                        detail: Detail::TagOrder,
                    },
                    Finding {
                        line: 9,
                        check: Check::Aliases,
                        detail: Detail::TagOrder,
                    },
                ],
                lines_checked: 10,
            }),
            ..Default::default()
        };
        w.sort = (0, true);
        w.sort_rows();
        assert_eq!(w.rows, vec![1, 0, 2]);
        w.sort = (0, false);
        w.sort_rows();
        assert_eq!(w.rows, vec![2, 0, 1]);
        w.sort = (2, true);
        w.sort_rows();
        assert_eq!(w.rows, vec![1, 2, 0], "by check, then line");
    }
}
