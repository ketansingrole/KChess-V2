//! Books: a PDF opened beside the board, its diagrams found and set up in a click, moves played
//! on them, the next diagram over the page's end, and the book reopened where it was left.

use super::Harness;

/// The export's picture of `fen` with cburnett pieces on the brown board: a book's diagram.
fn diagram(fen: &str) -> image::RgbImage {
    let options = kchess_board::game_image::ImageOptions {
        setup: kchess_domain::variant::Setup::standard_at(fen),
        moves: Vec::new(),
        orientation: shakmaty::Color::White,
        board_theme: "brown".into(),
        piece_set: "cburnett".into(),
        white: String::new(),
        black: String::new(),
        size: 400,
    };
    let png = kchess_board::game_image::position_png(&options, 0).expect("the picture");
    image::load_from_memory(&png).expect("a PNG").to_rgb8()
}

/// A PDF of `pages`, each 600 by 500 points with its diagrams side by side, as a PDF writer
/// writes one: the pictures as images, each page drawing its own.
fn book(pages: &[&[&str]]) -> Vec<u8> {
    let mut objects: Vec<Vec<u8>> = vec![b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(), Vec::new()];
    let mut kids = Vec::new();
    for diagrams in pages {
        let mut names = Vec::new();
        let mut drawing = String::new();
        for (k, fen) in diagrams.iter().enumerate() {
            let picture = diagram(fen);
            let mut image = format!(
                "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB \
                 /BitsPerComponent 8 /Length {} >>\nstream\n",
                picture.width(),
                picture.height(),
                picture.as_raw().len()
            )
            .into_bytes();
            image.extend_from_slice(picture.as_raw());
            image.extend_from_slice(b"\nendstream");
            objects.push(image);
            names.push(format!("/Im{k} {} 0 R", objects.len()));
            // 250 points wide, as tall as its own shape makes it: the export's picture has the
            // players' bars above and below the board.
            let height = 250 * picture.height() / picture.width();
            drawing.push_str(&format!(
                "q 250 0 0 {height} {} 100 cm /Im{k} Do Q\n",
                25 + 300 * k
            ));
        }
        objects.push(
            format!(
                "<< /Length {} >>\nstream\n{drawing}\nendstream",
                drawing.len()
            )
            .into_bytes(),
        );
        let contents = objects.len();
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 500] /Contents {contents} 0 R \
                 /Resources << /XObject << {} >> >> >>",
                names.join(" ")
            )
            .into_bytes(),
        );
        kids.push(format!("{} 0 R", objects.len()));
    }
    objects[1] = format!(
        "<< /Type /Pages /Kids [{}] /Count {} >>",
        kids.join(" "),
        kids.len()
    )
    .into_bytes();
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (k, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", k + 1).bytes());
        out.extend(object);
        out.extend(b"\nendobj\n");
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

fn wait_for_fen(app: &mut Harness, fen: &str) {
    let fen = fen.to_string();
    app.wait_for(&format!("the board at {fen}"), move |_, cx| {
        crate::shell::view::<crate::pages::books::BooksPage>(cx)
            .is_some_and(|page| page.read(cx).board_fen(cx) == fen)
    });
}

const ITALIAN: &str = "r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 0 1";
const ENDING: &str = "8/5k2/8/3K4/8/8/4P3/8 w - - 0 1";
const BACK_RANK: &str = "6k1/5ppp/8/8/8/8/5PPP/3R2K1 w - - 0 1";

/// A book opened from the Open dialog shows its first page with its diagrams outlined; a click
/// sets one up on the board, where its moves are played and taken back; Next diagram goes on to
/// the page's next and then over a page without diagrams to the next one's first, and back
/// the same way; the side to move can be switched. Reopened, the page opens the book again where it was left.
#[gpui_kit::test]
fn a_book_s_diagrams_are_set_up_on_the_board(cx: &mut gpui_kit::TestAppContext) {
    let folder = tempfile::tempdir().expect("a folder");
    let path = folder.path().join("Tactics.pdf");
    std::fs::write(&path, book(&[&[ITALIAN, ENDING], &[], &[BACK_RANK]]))
        .expect("the book written");

    let mut app = Harness::start(cx);
    app.open("Books");
    app.click("books-open-empty");
    app.wait_for_open_dialog();
    let chosen = path.clone();
    app.cx
        .simulate_path_prompt_response(move |_| Some(vec![chosen]));
    app.wait_for("both diagrams outlined", |window, _| {
        use gpui_kit::test::TestQueryExt as _;
        window.try_find_by_label("Diagram 2").is_some()
    });

    app.click(("book-diagram", 0usize));
    wait_for_fen(&mut app, ITALIAN);
    // Qxf7 is mate; taken back, the diagram is as it was.
    app.drag_square("h5", "f7", true);
    wait_for_fen(
        &mut app,
        "r1bqkb1r/pppp1Qpp/2n2n2/4p3/2B1P3/8/PPPP1PPP/RNB1K1NR b KQkq - 0 1",
    );
    app.click("books-take-back");
    wait_for_fen(&mut app, ITALIAN);

    app.click("books-next-diagram");
    wait_for_fen(&mut app, ENDING);
    app.click("books-side");
    wait_for_fen(&mut app, "8/5k2/8/3K4/8/8/4P3/8 b - - 0 1");
    // Past the page's last diagram: over the page of text alone to the next diagram's page.
    app.click("books-next-diagram");
    wait_for_fen(&mut app, BACK_RANK);
    let page = |app: &mut Harness| {
        app.cx.update(|cx| {
            crate::shell::view::<crate::pages::books::BooksPage>(cx)
                .map(|page| page.read(cx).page())
        })
    };
    assert_eq!(page(&mut app), Some(2));
    // And back: the last diagram of the page before the text.
    app.click("books-previous-diagram");
    wait_for_fen(&mut app, ENDING);
    assert_eq!(page(&mut app), Some(0));
    app.click("books-next-diagram");
    wait_for_fen(&mut app, BACK_RANK);
    assert_eq!(page(&mut app), Some(2));

    // Reopened, the book is open on the page it was left on.
    let mut app = app.restart();
    app.open("Books");
    app.wait_for("the book reopened on its third page", |window, cx| {
        use gpui_kit::test::TestQueryExt as _;
        window.try_find_by_label("Diagram 1").is_some()
            && crate::shell::view::<crate::pages::books::BooksPage>(cx)
                .is_some_and(|page| page.read(cx).page() == 2)
    });
    app.click(("book-diagram", 0usize));
    wait_for_fen(&mut app, BACK_RANK);
}

/// What is no PDF is refused with a line saying so, and the page stays ready to open another.
#[gpui_kit::test]
fn a_file_that_is_no_book_is_refused(cx: &mut gpui_kit::TestAppContext) {
    let folder = tempfile::tempdir().expect("a folder");
    let path = folder.path().join("notes.pdf");
    std::fs::write(&path, b"Not a PDF at all").expect("the file written");
    let mut app = Harness::start(cx);
    app.open("Books");
    app.click("books-open");
    app.wait_for_open_dialog();
    app.cx
        .simulate_path_prompt_response(move |_| Some(vec![path]));
    app.wait_for("the refusal", |_, cx| {
        crate::shell::view::<crate::pages::books::BooksPage>(cx).is_some_and(|page| {
            page.read(cx).problem().as_deref() == Some("That file is not a PDF KChess can read")
        })
    });
    assert!(app.shows("books-problem"));
    assert!(app.shows("books-open-empty"));
}
