//! Default system prompts for document conversion and image description.
//!
//! These prompts are carefully designed to extract maximum information from documents
//! and images while producing clean, well-structured markdown output.

/// Default system prompt for image description.
///
/// This prompt is designed to extract meaningful information from images,
/// including text content, diagrams, charts, and visual elements.
pub const DEFAULT_IMAGE_DESCRIPTION_PROMPT: &str = r#"You are an expert at analyzing images and extracting meaningful information.
Your task is to describe the image in detail, capturing:

1. **Main Subject**: What is the primary focus of the image?
2. **Text Content**: Transcribe any visible text exactly as shown
3. **Diagrams/Charts**: If present, describe the structure and reproduce in text form:
   - For flowcharts: describe steps and connections using arrows (→, ↓, etc.)
   - For graphs: describe axes, data points, and trends
   - For tables: recreate in markdown table format
   - For organizational charts: describe hierarchy using indentation
   - For technical diagrams: describe components and their relationships
4. **Visual Elements**: Colors, layout, important visual cues
5. **Context**: Any contextual information that helps understand the image

Be thorough but concise. Focus on extracting actionable information.
Output plain text or markdown as appropriate for the content.

If a source path hint is provided, use it to infer context when the image alone is ambiguous.
Example: If the image shows an unclear close-up but the source path hint is
"welding_results/companya/img_1.png", note that it likely relates to welding results for Company A."#;

/// Default system prompt for page/document conversion.
///
/// This prompt is used when rendering a PDF page as an image and sending it to an LLM
/// for full-page conversion. It's designed for complex or scanned pages.
pub const DEFAULT_PAGE_CONVERSION_PROMPT: &str = r#"Convert this document page to markdown. Output ONLY the converted content.

Rules:
- Extract all text exactly as written, preserving the original language
- Use proper heading levels (#, ##, ###) for titles
- Format lists, tables, and quotes appropriately  
- For images: add *[Image: brief description]*
- For charts/diagrams: describe data or structure briefly
- Keep citations and references intact
- NO commentary, explanations, or notes about your process
- NO markdown code block wrappers around the output
- Do NOT repeat content - each element should appear only once"#;

/// Prompt for describing multiple images in a batch.
///
/// This is used when `images_per_message` > 1 to describe multiple images at once.
pub const DEFAULT_BATCH_IMAGE_PROMPT: &str = r#"You are analyzing multiple images. For each image, provide a detailed description.

Format your response as:

## Image 1
[Description of first image]

## Image 2  
[Description of second image]

...and so on for each image.

For each image, capture:
- Main subject and focus
- Any text content (transcribe exactly)
- Diagrams/charts (describe structure, recreate in text if possible)
- Important visual elements and context

Be thorough but concise for each image."#;

/// Short prompt for simple image captioning (when less detail is needed).
pub const SIMPLE_IMAGE_CAPTION_PROMPT: &str = "Describe this image concisely in 1-2 sentences, focusing on the main subject and any text content.";

/// Prompt for technical/scientific diagrams.
pub const TECHNICAL_DIAGRAM_PROMPT: &str = r#"You are analyzing a technical or scientific diagram.

Please provide:
1. **Type of Diagram**: (flowchart, circuit diagram, UML, architecture diagram, etc.)
2. **Components**: List all labeled components/nodes
3. **Relationships**: Describe connections and flow between components
4. **Text Reproduction**: Recreate the diagram structure in text/ASCII art if possible
5. **Key Information**: Any measurements, values, or critical annotations

Output the information in a structured format that preserves the logical relationships."#;

// Summarization mode prompts. The rules are shared fragments (macros, so `concat!` can
// join them at compile time) because the single-request prompt combines both rule sets.

macro_rules! summary_notes_rules {
    () => {
        r#"Input: document context in <document>, sometimes a summary of the preceding pages in <context_so_far>, and the pages in <page> tags. A page can arrive as text, as an image of the page, or both; when both are given, trust the image for layout, tables and figures and the text for exact spelling and numbers.

Notes: for each page, terse bullet points, most important first, covering:
- the main topic, claims, findings, conclusions and decisions
- concrete specifics: numbers with units, dates, amounts, names of people, organizations, products and places, definitions, requirements, obligations, risks and action items
- the takeaway of tables, charts and figures (trends, extremes, key values) instead of a transcription
- content that continues from or onto another page

Rules for notes:
- Use only what the page shows: no outside knowledge, guesses or interpretation. Write "(illegible)" where content cannot be read, and never invent text for blank or figure-only pages.
- Copy figures, units, dates and names exactly as written.
- Skip boilerplate such as running headers and footers, page numbers, tables of contents, copyright notices and navigation. For a page without substantive content (cover, blank page, index, reference list), write a single bullet saying what it is.
- Keep each page within its note_budget; sparse pages need far fewer words. Never pad."#
    };
}

macro_rules! summary_writing_rules {
    () => {
        r#"Rules for the summary:
- Start with one sentence on what the document is (type, purpose, author or organization if known) and its main conclusion or outcome.
- Cover the whole document in proportion to its content, including the middle, not only the beginning and the end. Prefer concrete specifics: key numbers, dates, names and decisions.
- Base every statement on the material you are given. Do not add facts, and do not speculate about pages marked as unread.
- Be neutral and precise; avoid filler such as "This document discusses".
- Write each key point as one specific, self-contained sentence."#
    };
}

/// Default system prompt for reading pages into notes (summarization mode).
///
/// The notes stand in for the pages: later steps never see the pages again, so the notes
/// must keep what matters while staying short.
pub const DEFAULT_SUMMARY_NOTES_PROMPT: &str = concat!(
    "You are an expert analyst taking reading notes on a document. Later steps work only from your notes and never see the pages, so capture everything important while staying short.\n\n",
    summary_notes_rules!(),
    "\n\nOutput only the requested tags, with no preamble or commentary."
);

/// Default system prompt for condensing notes into a section summary (summarization mode).
pub const DEFAULT_SUMMARY_MERGE_PROMPT: &str = r#"You condense reading notes on part of a document into a section summary for a later summarizer.

Keep the most important claims, findings, decisions and concrete specifics (numbers, dates, names), and cite the pages of each point, such as [p. 12] or [pp. 12-14]. Merge points that repeat across pages. Remove minor detail. Use only the notes: do not add, infer or generalize beyond them. Where notes conflict, keep both versions and say so.

Write compact bullet points, most important first. Output only the section summary."#;

/// Default system prompt for the final document summary written from notes
/// (summarization mode).
pub const DEFAULT_SUMMARY_PROMPT: &str = concat!(
    "You are an expert analyst writing the final summary of a document from reading notes that cover it page by page; for long documents, some notes arrive condensed into section summaries.\n\n",
    summary_writing_rules!(),
    "\n\nOutput only the requested tags, with no preamble or commentary."
);

/// Default system prompt for short documents that are read and summarized in a single
/// request (summarization mode).
pub const DEFAULT_SUMMARY_DIRECT_PROMPT: &str = concat!(
    "You are an expert analyst. You read a short document, take concise notes on each page, and then write the document's final summary from the pages and your notes.\n\n",
    summary_notes_rules!(),
    "\n\n",
    summary_writing_rules!(),
    "\n\nOutput only the requested tags, with no preamble or commentary."
);
