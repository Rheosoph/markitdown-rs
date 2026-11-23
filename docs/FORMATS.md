# Supported Formats

**markitdown-rs** supports conversion of 40+ document, spreadsheet, presentation, ebook, and data formats to clean, readable Markdown.

## Quick Reference

### 📄 Documents
- **Word** (.docx, .doc) – Modern and legacy Microsoft Word documents
- **HTML** (.html, .htm) – Web pages and HTML documents
- **PDF** (.pdf) – Portable documents with text extraction
- **Markdown** (.md, .markdown) – Pass-through with normalization
- **LaTeX** (.tex, .latex) – Mathematical typesetting documents
- **Org-mode** (.org) – Emacs outline and note-taking format
- **reStructuredText** (.rst) – Sphinx and Python documentation
- **Typst** (.typ) – Modern typesetting language
- **DocBook** (.docbook, .dbk) – Structured XML documentation
- **RTF** (.rtf) – Rich Text Format documents

### 📊 Spreadsheets & Data
- **Excel** (.xlsx, .xls) – Modern and legacy spreadsheets
- **CSV** (.csv) – Comma-separated values
- **YAML** (.yaml, .yml) – Data serialization format
- **JSON** (.json) – JavaScript Object Notation
- **TOML** (.toml) – Configuration file format
- **SQLite** (.sqlite, .db) – Database files
- **Plain Text** (.txt) – Plain text files

### 🎙️ Presentations
- **PowerPoint** (.pptx, .ppt) – Modern and legacy presentations
- **PDF** (.pdf) – PDF slide decks

### 📚 Ebooks & Reference
- **EPUB** (.epub) – Electronic publications (E-books)
- **FictionBook** (.fb2) – FB2 Russian ebook format
- **DocBook** (.docbook) – Structured documents

### 📧 Communication
- **Email** (.eml, .msg) – Email messages and archives

### 🔗 Web & Feeds
- **RSS/Atom** (.xml) – News feeds and subscriptions

### 🎨 Images
- **Raster Images** (.png, .jpg, .jpeg, .bmp, .gif, .tiff, .webp) – Image files with optional OCR

### 📦 Archives & Containers
- **ZIP** (.zip) – ZIP archives
- **TAR** (.tar, .tar.gz, .tgz) – POSIX tar archives
- **Gzip** (.gz) – Gzip compressed files
- **Bzip2** (.bz2) – Bzip2 compressed files
- **XZ** (.xz) – XZ compressed files
- **Zstandard** (.zst) – Zstandard compressed files
- **iWork** (.pages, .numbers, .key) – Apple document containers (ZIP-based)

### 🗂️ Other Formats
- **OpenDocument** (.odt, .ods, .odp) – LibreOffice/OpenOffice documents
- **iCalendar** (.ics) – Calendar events
- **vCard** (.vcf) – Contact information
- **Jupyter Notebooks** (.ipynb) – Interactive Python notebooks
- **OPML** (.opml) – Outline Processor Markup Language
- **Log Files** (.log) – Application and system logs
- **BibTeX** (.bib) – Bibliography references

## Format Details

### Documents

#### Word Documents (.docx, .doc)
Converts Microsoft Word documents with support for:
- Text and formatting (bold, italic, underline)
- Headings and lists
- Tables (with optional multi-page table merging)
- Images (extractable): placed where the document shows them, with the alt text set in Word; unused media files and header images are left out
- Comments and tracked changes

#### HTML (.html, .htm)
Extracts content from web pages and HTML documents:
- Semantic HTML structure → Markdown headings/lists
- Tables with proper formatting
- Links and images: embedded images (data URLs) become images at their position, linked images stay links
- Code blocks

#### PDF (.pdf)
Text extraction from PDF files with:
- Layout preservation where possible
- Table detection and formatting
- Figures with `extract_images`: embedded images and cut-outs of vector drawings such as charts, after each page's text (logos, letterheads, slide templates, backgrounds and tables are left out)
- Multi-page support

#### LaTeX (.tex, .latex)
Converts LaTeX source documents:
- Sections and environments
- Mathematical expressions (preserved as-is)
- Lists and enumerations
- Code blocks

#### Org-mode (.org)
Emacs outline format with:
- Headlines (up to any level)
- Code blocks with syntax highlighting
- Links and cross-references
- Lists and checkboxes

### Spreadsheets & Data

#### Excel (.xlsx, .xls)
Converts spreadsheet files:
- Multiple sheets
- Tables and data formatting
- Merged cells handling
- Formula results (not formulas)

#### CSV (.csv)
Parses delimiter-separated values:
- Auto-detects delimiter
- Converts to Markdown tables

#### JSON/YAML/TOML (.json, .yaml, .yml, .toml)
Data serialization formats converted as structured text

### Presentations

#### PowerPoint (.pptx, .ppt)
Converts presentation slides:
- Slide content as separate pages, in presentation order (not file name order)
- Text, lists, and shapes, one line per paragraph
- Images from slides, with the alt text set in PowerPoint (background pictures are left out)
- Native charts as a table of their data
- Footer, date and slide number placeholders are left out
- Notes (when available)

### Ebooks

#### EPUB (.epub)
Electronic publications with:
- Chapter-by-chapter extraction
- Images and embedded media
- Metadata (title, author)
- Multi-file support

#### FictionBook (.fb2)
Russian ebook format supporting:
- Story structure and sections
- Embedded images
- Metadata and annotations

### Feeds & Web

#### RSS/Atom (.xml)
News feed subscription format:
- Feed title and description
- Individual entries with content
- Links preserved

### Images

#### Raster Images (.png, .jpg, .bmp, .gif, .tiff, .webp)
Image files can be:
- Extracted with metadata
- Optionally OCR'd for text extraction (with LLM)

### Archives

All archive formats extract and recursively convert their contents:
- Nested archives are handled
- Individual files are converted to Markdown

## Conversion Accuracy

- **Text-heavy formats** (Word, PDF, HTML): ~95%+ accuracy
- **Structured data** (CSV, JSON, YAML): Lossless conversion
- **Complex layouts** (PowerPoint, PDF with graphics): Best-effort approximation
- **Legacy formats** (.doc, .xls, .ppt): Supported with potential loss of modern features

## Known Limitations

1. **PDF**: Complex layouts with columns or heavily formatted content may lose visual structure
2. **Images**: Rasterized images are extracted but not described unless OCR is enabled
3. **Legacy Office**: Features not present in older Office formats (.doc, .xls, .ppt) are lost
4. **Formulas**: Excel formulas are evaluated, not preserved as formulas
5. **Styling**: Only semantic styling (bold, italic) is preserved; decorative formatting is lost

## Adding New Formats

To add support for a new format:

1. Create a new converter in `src/<format>.rs` implementing `DocumentConverter`
2. Register it in `src/lib.rs` with `register_converter()`
3. Add test fixtures to `tests/test_documents/<format>/`
4. Create a test file at `tests/<format>.rs`

See [CONTRIBUTING.md](../CONTRIBUTING.md) for detailed guidelines.
