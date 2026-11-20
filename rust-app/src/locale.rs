use std::sync::OnceLock;

fn is_korean_language(lang_id: u16) -> bool {
    const PRIMARY_MASK: u16 = 0x03ff;
    const KOREAN: u16 = 0x0012;
    lang_id & PRIMARY_MASK == KOREAN
}

pub fn korean() -> bool {
    static KOREAN: OnceLock<bool> = OnceLock::new();
    *KOREAN.get_or_init(|| {
        let lang = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
        is_korean_language(lang)
    })
}

macro_rules! tr {
    ($ko:expr, $en:expr) => {
        if $crate::locale::korean() {
            $ko
        } else {
            $en
        }
    };
}

macro_rules! trf {
    ($ko:literal, $en:literal $(, $args:expr)* $(,)?) => {
        if $crate::locale::korean() { format!($ko $(, $args)*) } else { format!($en $(, $args)*) }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn korean_ui_language_is_distinct_from_other_windows_languages() {
        assert!(is_korean_language(0x0412));
        assert!(!is_korean_language(0x0409));
        assert!(!is_korean_language(0x0411));
        assert!(!is_korean_language(0x0804));
    }
}
