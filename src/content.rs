use caldir_core::{Event, XProperty};

use crate::constants::ITEM_UID_PROPERTY;

pub fn normalize_color(color: &str) -> String {
    let color = color.strip_prefix('#').unwrap_or(color);
    let rgb = if color.len() == 8 { &color[..6] } else { color };
    if rgb.len() == 6 && rgb.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        format!("#{rgb}")
    } else {
        color.to_string()
    }
}

pub fn set_item_ref(event: &mut Event, item_ref: &str) {
    event
        .x_properties
        .retain(|property| !property.name.eq_ignore_ascii_case(ITEM_UID_PROPERTY));
    event
        .x_properties
        .push(XProperty::new(ITEM_UID_PROPERTY, item_ref));
}

pub fn item_ref(event: &Event) -> Option<&str> {
    event.x_properties.iter().find_map(|property| {
        property
            .name
            .eq_ignore_ascii_case(ITEM_UID_PROPERTY)
            .then_some(property.value.as_str())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use caldir_core::EventTime;
    use chrono::NaiveDate;

    #[test]
    fn normalizes_tuta_colors() {
        assert_eq!(normalize_color("8bc34a"), "#8bc34a");
        assert_eq!(normalize_color("#8bc34aff"), "#8bc34a");
        assert_eq!(normalize_color("green"), "green");
    }

    #[test]
    fn item_ref_is_deduplicated() {
        let mut event = Event::new(
            "Test",
            EventTime::Date(NaiveDate::from_ymd_opt(2026, 7, 18).unwrap()),
        );
        set_item_ref(&mut event, "old");
        set_item_ref(&mut event, "new");
        assert_eq!(item_ref(&event), Some("new"));
        assert_eq!(event.x_properties.len(), 1);
    }
}
