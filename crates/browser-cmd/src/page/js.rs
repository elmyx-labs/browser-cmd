//! The JavaScript run in the page, one function per file under `js/`. Each but
//! the canvas text recorder runs in an isolated world that shares the DOM but
//! not the page's globals, and takes what it needs as arguments: nothing is
//! spliced into its source.

macro_rules! scripts {
    ($($name:ident = $file:literal;)*) => {
        $(pub(crate) const $name: &str = include_str!(concat!("js/", $file));)*
    };
}

scripts! {
    CANVASES = "canvases.js";
    CLEAR_EDITABLE = "clear_editable.js";
    CLICKABLE = "clickable.js";
    CLIENT_HINTS = "client_hints.js";
    COMMIT = "commit.js";
    COVERED_BY = "covered_by.js";
    DOCUMENT = "document.js";
    DRAWN_TEXT = "drawn_text.js";
    EDITABLE = "editable.js";
    EVIDENCE = "evidence.js";
    EXTRACT = "extract.js";
    FIELD_VALUE = "field_value.js";
    IMAGES = "images.js";
    LIST = "list.js";
    LOCATE = "locate.js";
    MARKUP = "markup.js";
    OPTION = "option.js";
    OVERLAY = "overlay.js";
    PDF = "pdf.js";
    QUERY_SELECTOR = "query_selector.js";
    QUIET = "quiet.js";
    REACH = "reach.js";
    READ = "read.js";
    SCROLL_TO_TEXT = "scroll_to_text.js";
    SELECT_OPTION = "select_option.js";
    SET_VALUE = "set_value.js";
    SUBMITS = "submits.js";
    SUGGESTIONS = "suggestions.js";
    VIEWPORT = "viewport.js";
    WALL = "wall.js";
    WEBGL_RENDERER = "webgl_renderer.js";
}
