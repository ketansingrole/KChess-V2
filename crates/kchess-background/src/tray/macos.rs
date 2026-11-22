//! The menu bar item: an `NSStatusItem` whose button shows a template image (the menu bar tints
//! it for light, dark and highlighted), and whose `NSMenu` opens on a click. Each entry's action
//! is one method of a small target object, which reports the entry's tag.

// AppKit's status item, menu and target are reached through Objective-C calls.
#![allow(
    unsafe_code,
    reason = "AppKit's menu item targets and actions are unsafe to set; this module owns the menu bar item"
)]

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{
    AllocAnyThread as _, DefinedClass as _, MainThreadMarker, MainThreadOnly, define_class,
    msg_send, sel,
};
use objc2_app_kit::{
    NSControlStateValueOff, NSControlStateValueOn, NSImage, NSMenu, NSMenuItem, NSStatusBar,
    NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::{NSData, NSSize, NSString};

use super::{Event, Events, Icon, Item};

/// What the target reports to.
pub struct TargetIvars {
    events: Events,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and `Target` implements no `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "KChessTrayTarget"]
    #[ivars = TargetIvars]
    struct Target;

    // SAFETY: NSObjectProtocol has no safety requirements.
    unsafe impl NSObjectProtocol for Target {}

    impl Target {
        /// An entry was picked: its tag is the entry's id.
        // SAFETY: the selector takes the sending menu item and returns nothing.
        #[unsafe(method(chose:))]
        fn chose(&self, sender: &NSMenuItem) {
            if let Ok(id) = u32::try_from(sender.tag()) {
                (self.ivars().events)(Event::Chose(id));
            }
        }
    }
);

impl Target {
    fn new(events: Events, main: MainThreadMarker) -> Retained<Target> {
        let this = Target::alloc(main).set_ivars(TargetIvars { events });
        // SAFETY: NSObject's `init` on a freshly allocated object.
        unsafe { msg_send![super(this), init] }
    }
}

pub struct Tray {
    item: Retained<NSStatusItem>,
    target: Retained<Target>,
    main: MainThreadMarker,
}

impl Tray {
    pub fn new(
        tooltip: &str,
        icon: &Icon,
        menu: Vec<Item>,
        events: Events,
    ) -> Result<Tray, String> {
        let main = MainThreadMarker::new().ok_or("The menu bar item needs the main thread.")?;
        let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        let button = item
            .button(main)
            .ok_or("The menu bar item has no button.")?;
        let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(icon.template_png))
            .ok_or("The menu bar icon could not be read.")?;
        // The menu bar's own size for an icon; a template follows its light, dark and
        // highlighted looks.
        image.setSize(NSSize::new(18.0, 18.0));
        image.setTemplate(true);
        button.setImage(Some(&image));
        button.setToolTip(Some(&NSString::from_str(tooltip)));
        let mut tray = Tray {
            item,
            target: Target::new(events, main),
            main,
        };
        tray.set_menu(menu);
        Ok(tray)
    }

    pub fn set_menu(&mut self, menu: Vec<Item>) {
        let built = self.menu(&menu);
        self.item.setMenu(Some(&built));
    }

    fn menu(&self, items: &[Item]) -> Retained<NSMenu> {
        let menu = NSMenu::new(self.main);
        // Entries are enabled as the items say, not by AppKit's validation of their target.
        menu.setAutoenablesItems(false);
        for item in items {
            menu.addItem(&self.entry(item));
        }
        menu
    }

    fn entry(&self, item: &Item) -> Retained<NSMenuItem> {
        match item {
            Item::Separator => NSMenuItem::separatorItem(self.main),
            Item::Submenu { label, items } => {
                let entry = self.titled(label, false);
                entry.setSubmenu(Some(&self.menu(items)));
                entry
            }
            Item::Action {
                id,
                label,
                enabled,
                checked,
            } => {
                let entry = self.titled(label, *enabled);
                entry.setTag(*id as isize);
                entry.setEnabled(*enabled);
                entry.setState(if *checked == Some(true) {
                    NSControlStateValueOn
                } else {
                    NSControlStateValueOff
                });
                entry
            }
        }
    }

    /// An entry titled `label`, reporting to the target when `picks`.
    fn titled(&self, label: &str, picks: bool) -> Retained<NSMenuItem> {
        let action = picks.then_some(sel!(chose:));
        // SAFETY: the selector is the target's `chose:`, which takes the menu item.
        let entry = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(self.main),
                &NSString::from_str(label),
                action,
                &NSString::new(),
            )
        };
        if picks {
            let target: &AnyObject = &self.target;
            // SAFETY: the target lives as long as the tray, which owns the menu.
            unsafe { entry.setTarget(Some(target)) };
        }
        entry
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        NSStatusBar::systemStatusBar().removeStatusItem(&self.item);
    }
}
