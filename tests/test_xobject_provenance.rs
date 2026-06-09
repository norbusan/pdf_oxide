//! Tests for per-character / per-path content provenance (Ask 1).
//!
//! `extract_chars` and `extract_paths` tag each item with `xobject_path`:
//! the Form XObject nesting chain (by resource name, outermost first) the
//! operator was emitted inside, or `None` for page-stream content. This
//! lets a consumer separate figure-local content (axis labels, frame
//! strokes inside a `/FigN` XObject) from genuine document-body content —
//! the key signal for hidden-text / obfuscation detection.

use pdf_oxide::document::PdfDocument;

// ---------------------------------------------------------------------------
// Helper: build a PDF whose page draws:
//   - body text "BODY" directly in the page content stream,
//   - Form XObject /Fig1 (invoked via `Do`) which itself:
//       * shows text "LABEL",
//       * strokes a path (a rectangle),
//       * invokes a nested Form XObject /Inner via `Do`,
//   - the nested Form /Inner shows text "NESTED".
//
// Expected provenance:
//   "BODY"   chars -> xobject_path == None        (page stream)
//   "LABEL"  chars -> ["Fig1"]                     (depth 1)
//   "NESTED" chars -> ["Fig1", "Inner"]            (depth 2)
//   page path        -> None
//   /Fig1 rect path  -> ["Fig1"]
// ---------------------------------------------------------------------------
fn build_provenance_pdf() -> Vec<u8> {
    let mut pdf = Vec::new();
    let mut offsets: Vec<usize> = Vec::new();

    pdf.extend_from_slice(b"%PDF-1.4\n");

    // Object 1: Catalog
    offsets.push(pdf.len());
    pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n\n");

    // Object 2: Pages
    offsets.push(pdf.len());
    pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n\n");

    // Object 3: Page. Resources expose the font /F1 and the top-level
    // Form XObject /Fig1.
    offsets.push(pdf.len());
    pdf.extend_from_slice(
        b"3 0 obj\n\
          << /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792]\n\
             /Contents 4 0 R\n\
             /Resources << /Font << /F1 7 0 R >> /XObject << /Fig1 5 0 R >> >>\n\
          >>\nendobj\n\n",
    );

    // Object 4: Page content stream.
    //   - "BODY" text directly in the page stream
    //   - a stroked line directly in the page stream
    //   - invoke /Fig1
    // "PQR" is shown AFTER returning from /Fig1 Do; its letters (P,Q,R) are
    // unique to this run, so it cleanly proves the provenance stack unwound
    // back to page scope after the nested forms were walked.
    let page_content = b"BT /F1 12 Tf 100 700 Td (BODY) Tj ET\n\
                         q 0 0 m 50 50 l S Q\n\
                         q 1 0 0 1 0 0 cm /Fig1 Do Q\n\
                         BT /F1 12 Tf 100 650 Td (PQR) Tj ET";
    offsets.push(pdf.len());
    let hdr = format!("4 0 obj\n<< /Length {} >>\nstream\n", page_content.len());
    pdf.extend_from_slice(hdr.as_bytes());
    pdf.extend_from_slice(page_content);
    pdf.extend_from_slice(b"\nendstream\nendobj\n\n");

    // Object 5: Form XObject /Fig1.
    //   - "LABEL" text
    //   - a stroked rectangle path
    //   - invoke nested Form /Inner
    let fig1_stream = b"BT /F1 12 Tf 20 20 Td (LABEL) Tj ET\n\
                        q 10 10 100 80 re S Q\n\
                        /Inner Do";
    offsets.push(pdf.len());
    let fig1_hdr = format!(
        "5 0 obj\n\
         << /Type /XObject /Subtype /Form /BBox [0 0 300 300]\n\
            /Resources << /Font << /F1 7 0 R >> /XObject << /Inner 6 0 R >> >>\n\
            /Length {} >>\nstream\n",
        fig1_stream.len()
    );
    pdf.extend_from_slice(fig1_hdr.as_bytes());
    pdf.extend_from_slice(fig1_stream);
    pdf.extend_from_slice(b"\nendstream\nendobj\n\n");

    // Object 6: nested Form XObject /Inner.
    let inner_stream = b"BT /F1 12 Tf 30 30 Td (NESTED) Tj ET";
    offsets.push(pdf.len());
    let inner_hdr = format!(
        "6 0 obj\n\
         << /Type /XObject /Subtype /Form /BBox [0 0 200 200]\n\
            /Resources << /Font << /F1 7 0 R >> >>\n\
            /Length {} >>\nstream\n",
        inner_stream.len()
    );
    pdf.extend_from_slice(inner_hdr.as_bytes());
    pdf.extend_from_slice(inner_stream);
    pdf.extend_from_slice(b"\nendstream\nendobj\n\n");

    // Object 7: Font
    offsets.push(pdf.len());
    pdf.extend_from_slice(
        b"7 0 obj\n\
          << /Type /Font /Subtype /Type1 /BaseFont /Helvetica\n\
             /Encoding /WinAnsiEncoding >>\nendobj\n\n",
    );

    // xref table
    let xref_offset = pdf.len();
    let n_obj = offsets.len() + 1;
    let mut xref = format!("xref\n0 {}\n", n_obj);
    xref.push_str("0000000000 65535 f \n");
    for off in &offsets {
        xref.push_str(&format!("{:010} 00000 n \n", off));
    }
    pdf.extend_from_slice(xref.as_bytes());

    let trailer = format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
        n_obj, xref_offset
    );
    pdf.extend_from_slice(trailer.as_bytes());

    pdf
}

/// Collect the `xobject_path` (as a `Vec<String>`) for the first character
/// of the first run whose text contains `needle`.
fn provenance_of(chars: &[pdf_oxide::layout::TextChar], needle: char) -> Option<Vec<String>> {
    chars
        .iter()
        .find(|c| c.char == needle)
        .map(|c| match &c.xobject_path {
            Some(chain) => chain.to_vec(),
            None => Vec::new(),
        })
}

#[test]
fn body_text_has_no_xobject_provenance() {
    let _ = env_logger::builder().is_test(true).try_init();
    let doc = PdfDocument::from_bytes(build_provenance_pdf()).expect("parse PDF");
    let chars = doc.extract_chars(0).expect("extract chars");

    // 'B' from "BODY" is page-stream content.
    let body = chars
        .iter()
        .find(|c| c.char == 'B')
        .expect("found 'B' from BODY");
    assert!(
        body.xobject_path.is_none(),
        "page-stream char must have xobject_path == None, got {:?}",
        body.xobject_path
    );
}

#[test]
fn figure_text_carries_single_level_provenance() {
    let _ = env_logger::builder().is_test(true).try_init();
    let doc = PdfDocument::from_bytes(build_provenance_pdf()).expect("parse PDF");
    let chars = doc.extract_chars(0).expect("extract chars");

    // 'L' from "LABEL" lives inside /Fig1 only.
    let path = provenance_of(&chars, 'L').expect("found 'L' from LABEL");
    assert_eq!(path, vec!["Fig1".to_string()], "LABEL char should have provenance [\"Fig1\"]");
}

#[test]
fn nested_xobject_text_carries_full_chain() {
    let _ = env_logger::builder().is_test(true).try_init();
    let doc = PdfDocument::from_bytes(build_provenance_pdf()).expect("parse PDF");
    let chars = doc.extract_chars(0).expect("extract chars");

    // 'N' from "NESTED" lives inside /Fig1 -> /Inner.
    let path = provenance_of(&chars, 'N').expect("found 'N' from NESTED");
    assert_eq!(
        path,
        vec!["Fig1".to_string(), "Inner".to_string()],
        "NESTED char should have provenance [\"Fig1\", \"Inner\"]"
    );
}

#[test]
fn provenance_stack_unwinds_after_returning_from_xobject() {
    // After walking into /Fig1 (and the nested /Inner) the extractor must
    // restore the page scope. "PQR" is shown after `/Fig1 Do`, so its chars
    // must be back to page-stream (None) — proving the stack unwound rather
    // than leaking the form scope.
    let _ = env_logger::builder().is_test(true).try_init();
    let doc = PdfDocument::from_bytes(build_provenance_pdf()).expect("parse PDF");
    let chars = doc.extract_chars(0).expect("extract chars");

    for needle in ['P', 'Q', 'R'] {
        let c = chars
            .iter()
            .find(|c| c.char == needle)
            .unwrap_or_else(|| panic!("found {:?} from PQR", needle));
        assert!(
            c.xobject_path.is_none(),
            "char {:?} shown after returning from /Fig1 must have None provenance, got {:?}",
            needle,
            c.xobject_path
        );
    }
}

#[test]
fn paths_carry_provenance() {
    let _ = env_logger::builder().is_test(true).try_init();
    let doc = PdfDocument::from_bytes(build_provenance_pdf()).expect("parse PDF");
    let paths = doc.extract_paths(0).expect("extract paths");

    assert!(!paths.is_empty(), "expected at least one path");

    // At least one page-stream path (the line) with no provenance.
    let has_page_path = paths.iter().any(|p| p.xobject_path.is_none());
    // At least one path inside /Fig1 (the stroked rectangle).
    let fig1_path = paths.iter().find(|p| {
        p.xobject_path
            .as_ref()
            .map(|c| c.as_ref() == ["Fig1".to_string()])
            .unwrap_or(false)
    });

    assert!(has_page_path, "expected a page-stream path with xobject_path == None");
    assert!(
        fig1_path.is_some(),
        "expected a path inside /Fig1 with provenance [\"Fig1\"], got {:?}",
        paths.iter().map(|p| &p.xobject_path).collect::<Vec<_>>()
    );
}
