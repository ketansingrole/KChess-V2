//! The tray item over D-Bus: a StatusNotifierItem with its menu (`com.canonical.dbusmenu`), which
//! KDE, GNOME with AppIndicator support (Ubuntu's default), Cinnamon, XFCE and others show. The
//! service runs on its own thread; a click reaches the app through `Events`.

use super::{Event, Events, Icon, Item};

/// What the D-Bus service reads to answer the tray host.
struct Shown {
    tooltip: String,
    pixmaps: Vec<ksni::Icon>,
    menu: Vec<Item>,
    events: Events,
}

impl ksni::Tray for Shown {
    fn id(&self) -> String {
        "kchess".into()
    }

    fn title(&self) -> String {
        self.tooltip.clone()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::ApplicationStatus
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.pixmaps.clone()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: self.tooltip.clone(),
            ..ksni::ToolTip::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        (self.events)(Event::Open);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        entries(&self.menu)
    }
}

/// A label as dbusmenu shows it: a single underscore marks the access key, so each is doubled.
fn label(text: &str) -> String {
    text.replace('_', "__")
}

fn entries(items: &[Item]) -> Vec<ksni::MenuItem<Shown>> {
    use ksni::menu::{CheckmarkItem, StandardItem, SubMenu};
    items
        .iter()
        .map(|item| match item {
            Item::Separator => ksni::MenuItem::Separator,
            Item::Submenu { label: text, items } => SubMenu {
                label: label(text),
                submenu: entries(items),
                ..SubMenu::default()
            }
            .into(),
            Item::Action {
                id,
                label: text,
                enabled,
                checked: Some(checked),
            } => {
                let id = *id;
                CheckmarkItem {
                    label: label(text),
                    enabled: *enabled,
                    checked: *checked,
                    activate: Box::new(move |shown: &mut Shown| (shown.events)(Event::Chose(id))),
                    ..CheckmarkItem::default()
                }
                .into()
            }
            Item::Action {
                id,
                label: text,
                enabled,
                checked: None,
            } => {
                let id = *id;
                StandardItem {
                    label: label(text),
                    enabled: *enabled,
                    activate: Box::new(move |shown: &mut Shown| (shown.events)(Event::Chose(id))),
                    ..StandardItem::default()
                }
                .into()
            }
        })
        .collect()
}

/// The icon's pixels as the tray reads them: ARGB, each pixel's bytes in network order.
fn pixmap(png_bytes: &[u8]) -> Result<ksni::Icon, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().map_err(|cause| cause.to_string())?;
    let size = reader
        .output_buffer_size()
        .ok_or("The tray icon is too large.")?;
    let mut buffer = vec![0; size];
    let frame = reader
        .next_frame(&mut buffer)
        .map_err(|cause| cause.to_string())?;
    if frame.color_type != png::ColorType::Rgba || frame.bit_depth != png::BitDepth::Eight {
        return Err("The tray icon is not 8-bit RGBA.".into());
    }
    let data = buffer[..frame.buffer_size()]
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|&[r, g, b, a]| [a, r, g, b])
        .collect();
    Ok(ksni::Icon {
        width: i32::try_from(frame.width).map_err(|cause| cause.to_string())?,
        height: i32::try_from(frame.height).map_err(|cause| cause.to_string())?,
        data,
    })
}

pub struct Tray(ksni::blocking::Handle<Shown>);

impl Tray {
    pub fn new(
        tooltip: &str,
        icon: &Icon,
        menu: Vec<Item>,
        events: Events,
    ) -> Result<Tray, String> {
        use ksni::blocking::TrayMethods as _;
        let shown = Shown {
            tooltip: tooltip.to_string(),
            pixmaps: vec![pixmap(icon.color_png)?],
            menu,
            events,
        };
        // A Snap or Flatpak may not own the item's well-known bus name; the tray finds the item
        // by its connection either way.
        let sandboxed = kchess_host::env::var_os("SNAP").is_some()
            || kchess_host::env::var_os("FLATPAK_ID").is_some();
        shown
            .disable_dbus_name(sandboxed)
            .spawn()
            .map(Tray)
            .map_err(|cause| format!("No system tray is available: {cause}"))
    }

    pub fn set_menu(&mut self, menu: Vec<Item>) {
        self.0.update(|shown| shown.menu = menu);
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        // The item leaves the tray; the service's thread ends on its own.
        drop(self.0.shutdown());
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_icon_is_read_as_argb() {
        // One opaque red pixel and one half-transparent blue one.
        let mut png_bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_bytes, 2, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&[255, 0, 0, 255, 0, 0, 255, 128])
                .unwrap();
        }
        let icon = super::pixmap(&png_bytes).unwrap();
        assert_eq!((icon.width, icon.height), (2, 1));
        assert_eq!(icon.data, [255, 255, 0, 0, 128, 0, 0, 255]);
    }

    #[test]
    fn underscores_show_as_written() {
        assert_eq!(super::label("Play @a_b"), "Play @a__b");
    }
}
