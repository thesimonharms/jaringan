use jaringan_core::Block;

const DYNAMIC_INCLUDE: &str = include_str!("../../../examples/scripts/dynamic-include.jrg");
const DATA_ENRICHMENT: &str = include_str!("../../../examples/scripts/data-enrichment.jrg");
const FORM_PROCESSOR: &str = include_str!("../../../examples/scripts/form-processor.jrg");

#[test]
fn parse_dynamic_include_jrg() {
    let doc = jaringan_core::parse_document(DYNAMIC_INCLUDE).unwrap();
    let scripts: Vec<_> = doc.blocks.iter().filter_map(|b| {
        if let Block::Script { wasm, label } = b {
            Some((label.clone(), wasm.len()))
        } else {
            None
        }
    }).collect();
    assert_eq!(scripts.len(), 1, "should have 1 Script block");
    assert!(scripts[0].1 > 0, "WASM must not be empty");
    assert_eq!(scripts[0].0.as_deref(), Some("Dynamic Include"));
    // Verify there's an "include:" paragraph
    assert!(doc.blocks.iter().any(|b| {
        matches!(b, Block::Paragraph(t) if t.contains("include:"))
    }), "should have include: paragraph");
}

#[test]
fn parse_data_enrichment_jrg() {
    let doc = jaringan_core::parse_document(DATA_ENRICHMENT).unwrap();
    let scripts: Vec<_> = doc.blocks.iter().filter_map(|b| {
        if let Block::Script { wasm, .. } = b {
            Some(wasm.len())
        } else {
            None
        }
    }).collect();
    assert_eq!(scripts.len(), 1);
    assert!(scripts[0] > 0);
}

#[test]
fn parse_form_processor_jrg() {
    let doc = jaringan_core::parse_document(FORM_PROCESSOR).unwrap();
    let scripts: Vec<_> = doc.blocks.iter().filter_map(|b| {
        if let Block::Script { wasm, .. } = b {
            Some(wasm.len())
        } else {
            None
        }
    }).collect();
    assert_eq!(scripts.len(), 1);
    assert!(scripts[0] > 0);
    // Verify input blocks are present
    let inputs: Vec<_> = doc.blocks.iter().filter_map(|b| {
        if let Block::Input(i) = b {
            Some(i.name.clone())
        } else {
            None
        }
    }).collect();
    assert_eq!(inputs, vec!["name", "email"]);
}
