#![no_main]

use clipt9n::llm::templates::{render, TemplateContext, TemplateKind, Templates};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 16 * 1024 {
        return;
    }
    if let Ok(source) = std::str::from_utf8(data) {
        if let Ok(t) = Templates::from_sources_for_test(source, "", "", "") {
            let ctx = TemplateContext::for_translate("German", "GLOSSARY");
            let _ = render(&t, TemplateKind::Translate, &ctx);
        }
    }
});
