pub const PARSER_UTF8: &str = "utf8-plain-v1";
pub const PARSER_PDF: &str = "pdf-extract-0.12.0";
pub const PARSER_DOCX: &str = "docx-rs-0.4.22";

pub const MEDIA_PLAIN: &str = "text/plain";
pub const MEDIA_MARKDOWN: &str = "text/markdown";
pub const MEDIA_PDF: &str = "application/pdf";
pub const MEDIA_DOCX: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseBudget {
    pub max_text_chars: usize,
    pub max_ms: u64,
}

pub const DEFAULT_PARSE_BUDGET: ParseBudget = ParseBudget {
    max_text_chars: 200_000,
    max_ms: 8_000,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    NotUtf8,
    NoTextLayer,
    ParseFailed,
    BudgetExceeded,
}

pub fn parser_version(media_kind: &str) -> &'static str {
    match media_kind {
        MEDIA_PDF => PARSER_PDF,
        MEDIA_DOCX => PARSER_DOCX,
        _ => PARSER_UTF8,
    }
}

pub fn extract_text(media_kind: &str, bytes: &[u8]) -> Result<String, ParseError> {
    extract_text_with_budget(media_kind, bytes, DEFAULT_PARSE_BUDGET)
}

pub fn extract_text_with_budget(
    media_kind: &str,
    bytes: &[u8],
    budget: ParseBudget,
) -> Result<String, ParseError> {
    let media_kind = media_kind.to_owned();
    let bytes = bytes.to_vec();
    run_with_budget(budget, move || extract_text_unbounded(&media_kind, &bytes))
}

fn extract_text_unbounded(media_kind: &str, bytes: &[u8]) -> Result<String, ParseError> {
    match media_kind {
        MEDIA_PLAIN | MEDIA_MARKDOWN => std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| ParseError::NotUtf8),
        MEDIA_PDF => extract_pdf(bytes),
        MEDIA_DOCX => extract_docx(bytes),
        _ => Err(ParseError::ParseFailed),
    }
}

fn run_with_budget(
    budget: ParseBudget,
    work: impl FnOnce() -> Result<String, ParseError> + Send + 'static,
) -> Result<String, ParseError> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("material-parse".into())
        .spawn(move || {
            let _ = tx.send(work());
        })
        .map_err(|_| ParseError::ParseFailed)?;
    match rx.recv_timeout(std::time::Duration::from_millis(budget.max_ms)) {
        Ok(Ok(text)) if text.chars().count() > budget.max_text_chars => {
            Err(ParseError::BudgetExceeded)
        }
        Ok(result) => result,
        Err(_) => Err(ParseError::BudgetExceeded),
    }
}

#[cfg(test)]
fn extract_with_parser(
    budget: ParseBudget,
    parser: impl FnOnce() -> Result<String, ParseError> + Send + 'static,
) -> Result<String, ParseError> {
    run_with_budget(budget, parser)
}

fn extract_pdf(bytes: &[u8]) -> Result<String, ParseError> {
    let text = pdf_extract::extract_text_from_mem(bytes).map_err(|_| ParseError::ParseFailed)?;
    if text.trim().is_empty() {
        Err(ParseError::NoTextLayer)
    } else {
        Ok(text)
    }
}

fn extract_docx(bytes: &[u8]) -> Result<String, ParseError> {
    let docx = docx_rs::read_docx(bytes).map_err(|_| ParseError::ParseFailed)?;
    let mut text = String::new();
    for child in &docx.document.children {
        append_document_child(&mut text, child);
    }
    if text.trim().is_empty() {
        Err(ParseError::NoTextLayer)
    } else {
        Ok(text)
    }
}

fn append_document_child(text: &mut String, child: &docx_rs::DocumentChild) {
    match child {
        docx_rs::DocumentChild::Paragraph(paragraph) => append_paragraph_text(text, paragraph),
        docx_rs::DocumentChild::Table(table) => append_table_text(text, table),
        docx_rs::DocumentChild::StructuredDataTag(tag) => append_sdt_text(text, tag),
        _ => {}
    }
}

fn append_table_text(text: &mut String, table: &docx_rs::Table) {
    for docx_rs::TableChild::TableRow(row) in &table.rows {
        for docx_rs::TableRowChild::TableCell(cell) in &row.cells {
            for content in &cell.children {
                match content {
                    docx_rs::TableCellContent::Paragraph(paragraph) => {
                        append_paragraph_text(text, paragraph);
                    }
                    docx_rs::TableCellContent::Table(nested) => append_table_text(text, nested),
                    docx_rs::TableCellContent::StructuredDataTag(tag) => append_sdt_text(text, tag),
                    _ => {}
                }
            }
        }
    }
}

fn append_paragraph_text(text: &mut String, paragraph: &docx_rs::Paragraph) {
    let start = text.len();
    append_paragraph_children(text, &paragraph.children);
    if text.len() > start {
        text.push('\n');
    }
}

fn append_paragraph_children(text: &mut String, children: &[docx_rs::ParagraphChild]) {
    for child in children {
        match child {
            docx_rs::ParagraphChild::Run(run) => append_run_text(text, run),
            docx_rs::ParagraphChild::Hyperlink(link) => {
                append_paragraph_children(text, &link.children);
            }
            docx_rs::ParagraphChild::Insert(insert) => append_insert_text(text, insert),
            docx_rs::ParagraphChild::StructuredDataTag(tag) => append_sdt_text(text, tag),
            _ => {}
        }
    }
}

fn append_insert_text(text: &mut String, insert: &docx_rs::Insert) {
    for child in &insert.children {
        if let docx_rs::InsertChild::Run(run) = child {
            append_run_text(text, run);
        }
    }
}

fn append_run_text(text: &mut String, run: &docx_rs::Run) {
    for run_child in &run.children {
        if let docx_rs::RunChild::Text(value) = run_child {
            text.push_str(&value.text);
        }
    }
}

fn append_sdt_text(text: &mut String, tag: &docx_rs::StructuredDataTag) {
    for child in &tag.children {
        match child {
            docx_rs::StructuredDataTagChild::Run(run) => append_run_text(text, run),
            docx_rs::StructuredDataTagChild::Paragraph(paragraph) => {
                append_paragraph_text(text, paragraph);
            }
            docx_rs::StructuredDataTagChild::Table(table) => append_table_text(text, table),
            docx_rs::StructuredDataTagChild::StructuredDataTag(nested) => {
                append_sdt_text(text, nested);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MEDIA_DOCX, MEDIA_MARKDOWN, MEDIA_PDF, MEDIA_PLAIN, ParseBudget, ParseError, extract_text,
        extract_text_with_budget, extract_with_parser, parser_version,
    };
    use std::path::PathBuf;

    fn fixture(name: &str) -> Vec<u8> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/materials")
            .join(name);
        let bytes =
            std::fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        if name.ends_with(".pdf") {
            assert!(
                bytes.starts_with(b"%PDF-"),
                "{} lost its PDF header; checkout probably rewrote it as text",
                path.display()
            );
        }
        bytes
    }

    #[test]
    fn pdf_fixtures_keep_checked_in_bytes() {
        let cases = [
            (
                "chinese-tounicode.pdf",
                "7658f29be27a1585af037f06f5cae2204a81eb4a992b0f28eed8015fd771a707",
            ),
            (
                "scanned-image-only.pdf",
                "3d2dfd6f546b79cec224af14bf216adeb9430165893b49b38f2fa32090357ecb",
            ),
        ];
        for (name, expected) in cases {
            let digest = crate::materials::store::sha256_hex(&fixture(name));
            assert_eq!(digest, expected, "{name} was rewritten on checkout");
        }
    }

    #[test]
    fn extract_utf8_plain_and_markdown() {
        assert_eq!(
            extract_text(MEDIA_PLAIN, "工作经历\n".as_bytes()).unwrap(),
            "工作经历\n"
        );
        assert_eq!(
            extract_text(MEDIA_MARKDOWN, "# 标题".as_bytes()).unwrap(),
            "# 标题"
        );
        assert_eq!(
            extract_text(MEDIA_PLAIN, &[0xff, 0xfe, 0x00]).unwrap_err(),
            ParseError::NotUtf8
        );
        assert_eq!(parser_version(MEDIA_PLAIN), "utf8-plain-v1");
        assert_eq!(parser_version(MEDIA_MARKDOWN), "utf8-plain-v1");
    }

    #[test]
    fn extract_chinese_tounicode_pdf() {
        let text = extract_text(MEDIA_PDF, &fixture("chinese-tounicode.pdf")).unwrap();
        assert!(
            text.contains("工作经历"),
            "expected ToUnicode CJK, got {text:?}"
        );
        assert_eq!(parser_version(MEDIA_PDF), "pdf-extract-0.12.0");
    }

    #[test]
    fn extract_scanned_pdf_is_no_text_layer() {
        assert_eq!(
            extract_text(MEDIA_PDF, &fixture("scanned-image-only.pdf")).unwrap_err(),
            ParseError::NoTextLayer
        );
    }

    #[test]
    fn extract_encrypted_pdf_is_parse_failed() {
        assert_eq!(
            extract_text(MEDIA_PDF, &fixture("encrypted-stub.pdf")).unwrap_err(),
            ParseError::ParseFailed
        );
    }

    #[test]
    fn extract_corrupt_pdf_is_parse_failed() {
        assert_eq!(
            extract_text(MEDIA_PDF, &fixture("corrupt-truncated.pdf")).unwrap_err(),
            ParseError::ParseFailed
        );
    }

    #[test]
    fn extract_chinese_docx() {
        let text = extract_text(MEDIA_DOCX, &fixture("chinese-synthetic.docx")).unwrap();
        assert!(text.contains("工作经历"), "got {text:?}");
        assert!(text.contains("示例科技"), "got {text:?}");
        assert_eq!(parser_version(MEDIA_DOCX), "docx-rs-0.4.22");
    }

    #[test]
    fn extract_chinese_table_docx() {
        let text = extract_text(MEDIA_DOCX, &fixture("chinese-table.docx")).unwrap();
        assert!(
            text.contains("工作经历"),
            "expected table-cell text, got {text:?}"
        );
        assert!(
            text.contains("示例科技"),
            "expected hyperlink text, got {text:?}"
        );
        assert!(
            text.contains("工程师"),
            "expected insert text, got {text:?}"
        );
    }

    #[test]
    fn extract_garbage_docx_is_parse_failed() {
        assert_eq!(
            extract_text(MEDIA_DOCX, &fixture("corrupt-not-zip.docx")).unwrap_err(),
            ParseError::ParseFailed
        );
    }

    #[test]
    fn extract_empty_incomplete_docx_is_parse_failed() {
        assert_eq!(
            extract_text(MEDIA_DOCX, &fixture("empty-text.docx")).unwrap_err(),
            ParseError::ParseFailed
        );
    }

    #[test]
    fn extract_text_rejects_when_text_budget_is_exceeded() {
        let body = "负责订单服务与 Kafka 链路优化。".repeat(8);
        assert!(body.chars().count() > 8);
        assert_eq!(
            extract_text_with_budget(
                MEDIA_PLAIN,
                body.as_bytes(),
                ParseBudget {
                    max_text_chars: 8,
                    max_ms: 5_000,
                },
            )
            .unwrap_err(),
            ParseError::BudgetExceeded
        );
        assert!(
            extract_text(MEDIA_PLAIN, body.as_bytes())
                .unwrap()
                .contains("订单服务")
        );
    }

    #[test]
    fn extract_text_stops_waiting_when_time_budget_elapses() {
        let started = std::time::Instant::now();
        let error = extract_with_parser(
            ParseBudget {
                max_text_chars: 1_000,
                max_ms: 40,
            },
            || {
                std::thread::sleep(std::time::Duration::from_millis(800));
                Ok("slow-parse".into())
            },
        )
        .unwrap_err();
        assert_eq!(error, ParseError::BudgetExceeded);
        assert!(
            started.elapsed() < std::time::Duration::from_millis(400),
            "caller must return after the join timeout, got {:?}",
            started.elapsed()
        );
    }
}
