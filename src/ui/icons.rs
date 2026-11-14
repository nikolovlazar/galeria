//! Line icons, drawn on a 16-point grid in the element's text colour.

use rune::Icon;

macro_rules! icon {
    ($name:ident, $id:literal, $body:literal) => {
        pub static $name: Icon = Icon {
            name: $id,
            view: 16.0,
            body: $body,
        };
    };
}

// The sidebar.
icon!(
    NOW,
    "galeria-now",
    r#"<rect x="2.25" y="3.25" width="11.5" height="9.5" rx="1.6" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M4.4 10.6L6.9 7.9L8.8 9.9L10.2 8.6L11.7 10.6" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" fill="none"/><circle cx="10.6" cy="6.2" r="0.9" fill="currentColor"/>"#
);
icon!(
    BROWSE,
    "galeria-browse",
    r#"<rect x="2.5" y="2.5" width="4.6" height="4.6" rx="1.1" stroke="currentColor" stroke-width="1.2" fill="none"/><rect x="8.9" y="2.5" width="4.6" height="4.6" rx="1.1" stroke="currentColor" stroke-width="1.2" fill="none"/><rect x="2.5" y="8.9" width="4.6" height="4.6" rx="1.1" stroke="currentColor" stroke-width="1.2" fill="none"/><rect x="8.9" y="8.9" width="4.6" height="4.6" rx="1.1" stroke="currentColor" stroke-width="1.2" fill="none"/>"#
);
icon!(
    DISCOVER,
    "galeria-discover",
    r#"<circle cx="7.1" cy="7.1" r="4.1" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M10.2 10.2L13.3 13.3" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/><path d="M7.1 5.2V9M5.2 7.1H9" stroke="currentColor" stroke-width="1.1" stroke-linecap="round"/>"#
);
icon!(
    MUSEUM,
    "galeria-museum",
    r#"<path d="M2.4 6.1L8 2.6L13.6 6.1Z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/><path d="M4.2 7.6V11.4M8 7.6V11.4M11.8 7.6V11.4" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/><path d="M2.6 13.2H13.4" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>"#
);
icon!(
    ROTATION,
    "galeria-rotation",
    r#"<path d="M12.9 7.2A5 5 0 0 0 3.6 5.6M3.1 8.8A5 5 0 0 0 12.4 10.4" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" fill="none"/><path d="M3.3 3.2V5.8H5.9M12.7 12.8V10.2H10.1" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#
);
icon!(
    WALLPAPER,
    "galeria-wallpaper",
    r#"<rect x="2.1" y="3" width="11.8" height="7.9" rx="1.4" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M6.1 13.3H9.9M8 11.1V13.2" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>"#
);
icon!(
    PLATE,
    "galeria-plate",
    r#"<rect x="2.3" y="3.8" width="11.4" height="8.4" rx="1.3" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M4.6 6.5H9.6M4.6 8.6H11.2M4.6 10.2H8" stroke="currentColor" stroke-width="1.1" stroke-linecap="round"/>"#
);
icon!(
    LIBRARY,
    "galeria-library",
    r#"<path d="M2.4 5.1V11.6A1.2 1.2 0 0 0 3.6 12.8H12.4A1.2 1.2 0 0 0 13.6 11.6V6.3A1.2 1.2 0 0 0 12.4 5.1H8.3L6.9 3.5H3.6A1.2 1.2 0 0 0 2.4 4.7Z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/>"#
);
icon!(
    GENERAL,
    "galeria-general",
    r#"<path d="M2.8 5H13.2M2.8 11H13.2" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/><circle cx="6" cy="5" r="1.7" fill="currentColor"/><circle cx="10.4" cy="11" r="1.7" fill="currentColor"/>"#
);

// Controls.
icon!(
    CHEVRONS,
    "galeria-chevrons",
    r#"<path d="M5.2 6.4L8 3.6L10.8 6.4M5.2 9.6L8 12.4L10.8 9.6" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#
);
icon!(
    CLOSE,
    "galeria-close",
    r#"<path d="M4.2 4.2L11.8 11.8M11.8 4.2L4.2 11.8" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/>"#
);
icon!(
    CHECK,
    "galeria-check",
    r#"<path d="M3.6 8.4L6.6 11.3L12.4 4.9" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#
);
icon!(
    STAR,
    "galeria-star",
    r#"<path d="M8 2.3L9.7 5.9L13.6 6.4L10.7 9.1L11.5 13L8 11.1L4.5 13L5.3 9.1L2.4 6.4L6.3 5.9Z" fill="currentColor"/>"#
);
icon!(
    SEARCH,
    "galeria-search",
    r#"<circle cx="7" cy="7" r="3.9" stroke="currentColor" stroke-width="1.3" fill="none"/><path d="M10 10L13 13" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>"#
);
