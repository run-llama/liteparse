//! Structured HTML alternate-content decoding and projection into Word's block model.
mod dom;
mod lower;
mod style;
mod word;

use super::*;

type HtmlProjection = (Vec<Rc<XNode>>, std::collections::BTreeSet<String>);
type ImportError = (&'static str, &'static str);

impl Parser<'_> {
    pub(super) fn html_chunk(
        &self,
        bytes: &[u8],
        charset: Option<&str>,
        part: &str,
        host: &str,
    ) -> Result<HtmlProjection, ImportError> {
        let text = decode_html(bytes, charset).map_err(|e| ("ALTCHUNK_DECODE_FAILED", e))?;
        let limit = self.imports.borrow().nodes;
        let (root, count) = dom::parse(&text, limit).map_err(|e| ("ALTCHUNK_RESOURCE_LIMIT", e))?;
        let output = lower::lower(self, &root, part, host, limit.saturating_sub(count))
            .map_err(|e| ("ALTCHUNK_RESOURCE_LIMIT", e))?;
        fn nodes(node: &XNode) -> usize {
            1 + node.children.iter().map(|n| nodes(n)).sum::<usize>()
        }
        let generated: usize = output.nodes.iter().map(|n| nodes(n)).sum();
        let total = count
            .checked_add(generated)
            .filter(|n| *n <= limit)
            .ok_or(("ALTCHUNK_RESOURCE_LIMIT", "HTML projection budget exceeded"))?;
        self.imports.borrow_mut().nodes -= total;
        Ok((output.nodes, output.warnings))
    }
}

fn decode_html(bytes: &[u8], charset: Option<&str>) -> Result<String, &'static str> {
    if let Some((encoding, skip)) = encoding_rs::Encoding::for_bom(bytes) {
        return encoding
            .decode_without_bom_handling_and_without_replacement(&bytes[skip..])
            .map(|s| s.into_owned())
            .ok_or("HTML contains invalid encoded text");
    }
    let declared = charset
        .map(|label| {
            encoding_rs::Encoding::for_label(label.as_bytes())
                .ok_or("HTML uses an unknown character encoding")
        })
        .transpose()?;
    let encoding = if let Some(encoding) = declared {
        encoding
    } else {
        // HTML meta declarations are ASCII-compatible. Decode the bounded prefix only
        // to inspect attributes, then decode the complete payload with the chosen codec.
        let prefix = &bytes[..bytes.len().min(1024)];
        let (probe, _, _) = encoding_rs::WINDOWS_1252.decode(prefix);
        let (root, _) = dom::parse(&probe, 1024)?;
        fn meta(node: &dom::Handle) -> Option<String> {
            if node.tag() == "meta" {
                if let Some(value) = node.attr("charset") {
                    return Some(value);
                }
                if node
                    .attr("http-equiv")
                    .is_some_and(|v| v.eq_ignore_ascii_case("content-type"))
                {
                    let content = node.attr("content")?;
                    for p in content.split(';') {
                        if let Some((key, value)) = p.split_once('=')
                            && key.trim().eq_ignore_ascii_case("charset")
                        {
                            return Some(value.trim().trim_matches(['\'', '"']).into());
                        }
                    }
                }
            }
            node.children.borrow().iter().find_map(meta)
        }
        match meta(&root) {
            Some(label) => encoding_rs::Encoding::for_label(label.as_bytes())
                .ok_or("HTML uses an unknown meta encoding")?,
            None => encoding_rs::WINDOWS_1252,
        }
    };
    encoding
        .decode_without_bom_handling_and_without_replacement(bytes)
        .map(|s| s.into_owned())
        .ok_or("HTML contains invalid encoded text")
}

#[cfg(test)]
mod tests;
