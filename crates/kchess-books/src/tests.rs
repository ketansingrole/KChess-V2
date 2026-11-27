use super::*;

/// A PDF whose pages are `pages` (width, height, content stream), written out with its cross
/// reference table, as a PDF writer would.
fn pdf(pages: &[(u32, u32, &str)]) -> Vec<u8> {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            (0..pages.len())
                .map(|k| format!("{} 0 R", 3 + 2 * k))
                .collect::<Vec<_>>()
                .join(" "),
            pages.len()
        ),
    ];
    for (k, (width, height, content)) in pages.iter().enumerate() {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] /Contents {} 0 R >>",
            4 + 2 * k
        ));
        objects.push(format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ));
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (k, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{object}\nendobj\n", k + 1).bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
    for offset in offsets {
        out.extend(format!("{offset:010} 00000 n \n").bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .bytes(),
    );
    out
}

/// Each page is drawn at the width asked, on white, with what it draws where it draws it: PDF's
/// origin is the page's bottom left, a picture's its top left.
#[test]
fn pages_are_drawn_at_the_width_asked() {
    let book = Book::open(pdf(&[
        (200, 300, "0 0 0 rg 50 100 100 100 re f"),
        (400, 200, ""),
    ]))
    .unwrap();
    assert_eq!(book.page_count(), 2);
    assert_eq!(book.page_size(0), Some((200., 300.)));
    assert_eq!(book.page_size(1), Some((400., 200.)));
    assert_eq!(book.page_size(2), None);

    let page = book.render(0, 400).unwrap();
    assert_eq!(page.dimensions(), (400, 600));
    let at = |x, y| page.get_pixel(x, y).0;
    // The square's middle, 100 to 200 points up: 200 to 400 pixels from the bottom.
    assert_eq!(at(200, 300), [0, 0, 0, 255]);
    assert_eq!(at(10, 10), [255, 255, 255, 255]);
    assert_eq!(at(200, 590), [255, 255, 255, 255]);
    assert_eq!(at(200, 190), [255, 255, 255, 255]);
    assert_eq!(at(200, 210), [0, 0, 0, 255]);
    assert_eq!(at(200, 390), [0, 0, 0, 255]);
    assert_eq!(at(200, 410), [255, 255, 255, 255]);

    assert_eq!(book.render(1, 100).unwrap().dimensions(), (100, 50));
    assert!(book.render(2, 100).is_none());
    // As wide as `MAX_WIDTH` at most.
    assert_eq!(book.render(1, 100_000).unwrap().width(), MAX_WIDTH);
}

/// What is no book is refused, saying why.
#[test]
fn what_is_no_book_is_refused() {
    assert_eq!(
        Book::open(b"PK\x03\x04 a zip".to_vec()).unwrap_err(),
        Refusal::Unreadable
    );
    assert_eq!(Book::open(Vec::new()).unwrap_err(), Refusal::Unreadable);
    assert_eq!(Book::open(pdf(&[])).unwrap_err(), Refusal::Empty);
    assert_eq!(
        Refusal::Locked.to_string(),
        "That PDF is locked with a password"
    );
}

/// A book is cheap to clone and goes to another thread to be drawn.
#[test]
fn a_book_is_drawn_on_another_thread() {
    let book = Book::open(pdf(&[(100, 100, "0 0 0 rg 0 0 100 100 re f")])).unwrap();
    let copy = book.clone();
    let drawn = std::thread::spawn(move || copy.render(0, 50))
        .join()
        .unwrap()
        .unwrap();
    assert_eq!(drawn.get_pixel(25, 25).0, [0, 0, 0, 255]);
    assert_eq!(format!("{book:?}"), "Book { 1 pages }");
}
