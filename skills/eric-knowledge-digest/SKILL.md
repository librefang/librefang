---
name: knowledge-digest
description: "Converts textbooks or PDFs into personalized, multimodal interactive learning materials including handwritten notes, quiz webpages, slides, audio courses, and mind maps. Trigger: learning materials, convert textbook, study notes, quiz generation, slides from PDF, mind map, audio course."
---

# KnowledgeDigest — Unified Learning Content Converter

## Overview

KnowledgeDigest converts textbooks, PDFs, or topic descriptions into personalized, multimodal learning experiences. It analyzes source content, then generates any combination of: handwritten-style notes (PDF), interactive quiz webpages (HTML), slides (PDF+PPTX), mind maps (image+Mermaid), and audio courses (MP3). All output is adapted to the learner's grade level and interests.

## Workflow

### Phase 1: Gather User Input

1. Identify what the user has provided:
   - Uploaded PDF/textbook file (optional)
   - Topic/direction description
   - Grade level (elementary / middle school / high school / university / professional)
   - Expected output format(s)

2. If no PDF/textbook uploaded and no source materials specified (only topic/direction provided):
   - Ask user:
     - **Option A:** "I have materials, uploading now"
     - **Option B:** "No materials, please search and generate courseware about [topic]"
   - If user selects B:
     - Use search tools to collect authoritative materials on the topic
     - Organize into structured content, generate a basic courseware PDF
     - Send PDF to user for confirmation: "This is the basic material I compiled for [topic], please confirm if usable?"
     - Continue after user confirmation

3. **Default output formats** (if user does not specify): mindmap + slides (PDF only) + quiz

### Phase 2: Content Analysis

Parse the PDF or structured content to extract:

**Document Parsing:**
- Identify chapter structure (chapters, sections, subsections)
- Extract heading hierarchy and table of contents
- Identify body text, images, tables, formulas, and other elements

**Core Concept Extraction:**
- Identify core concepts and key terms in each chapter
- Extract definitions, theorems, formulas, and important content
- Mark difficult points and key knowledge

**Learning Objective Analysis:**
- Infer learning objectives for each chapter
- Identify prerequisite knowledge requirements
- Analyze dependencies between knowledge points

**Output structured analysis results in this format:**

```json
{
  "document_info": {
    "title": "Document title",
    "total_pages": 100,
    "language": "zh/en",
    "subject": "Subject area"
  },
  "chapters": [
    {
      "chapter_id": "1",
      "title": "Chapter title",
      "page_range": [1, 20],
      "sections": [
        {
          "section_id": "1.1",
          "title": "Section title",
          "core_concepts": ["Concept 1", "Concept 2"],
          "key_terms": [
            {"term": "Term", "definition": "Definition"}
          ],
          "learning_objectives": ["Objective 1", "Objective 2"],
          "difficulty": "easy/medium/hard",
          "prerequisites": ["Prerequisite knowledge"]
        }
      ]
    }
  ],
  "knowledge_graph": {
    "nodes": ["Concept node list"],
    "edges": [{"from": "Concept A", "to": "Concept B", "relation": "depends/contains/related"}]
  }
}
```

**Parsing Rules:**
1. Chapter Recognition — Identify hierarchy based on font size, bold, numbering, etc. Handle documents without clear chapter markers by logically segmenting.
2. Concept Extraction — Identify bolded, highlighted, boxed important content. Extract proper nouns and term definitions. Identify formulas and theorems.
3. Difficulty Assessment — Assess based on concept abstraction level, prerequisite knowledge, and content complexity.
4. Quality Assurance — Ensure all chapters identified, verify knowledge point coverage completeness, check accuracy of concept definitions.

### Phase 3: Generate Requested Formats

Based on user-selected output formats, generate each in sequence. For each format, follow the corresponding section below.

### Phase 4: Deliver Assets

After all generation is complete:
- Only return file paths, no previews allowed
- No inline display of images/PDFs/audio/video in conversation
- Audio/video files must not auto-play

Present to user using deliver_assets format:
```
<deliver_assets>
<item>
<path>file path</path>
</item>
</deliver_assets>
```

---

## Supported Output Formats

| Format | Output | Description |
|--------|--------|-------------|
| `notes` | `{topic}_notes.pdf` | Handwritten-style notes (annotated on original or generated from scratch) |
| `quiz` | `{topic}_quiz.html` | Minimalist interactive HTML quiz with instant feedback |
| `slides` | `{topic}_slides.pdf` + `{topic}_slides.pptx` | Visual slides |
| `mindmap` | `{topic}_mindmap.png` + Mermaid text | Mind map image |
| `audio` | `{topic}_audio.mp3` | Audio course in teacher-student dialogue format |
| `all` | All of the above | Generate every format |

---

## Personalization: Grade Level Adaptation

All generated content must be adapted to the learner's grade level:

| Grade | Language & Tone | Content Density | Visual Style |
|-------|----------------|-----------------|--------------|
| **Elementary** | Lively, simple Q&A, encouraging, story-style | Low density, more drawings, large font | Fun elements, bright colors, short text |
| **Middle school** | Guided questioning, moderate challenges, youth-oriented | Moderate, image-text combination, clear labels | Image-text combination, moderate information |
| **High school** | In-depth discussion, logical reasoning, appropriate academic tone | Higher density, logic diagrams | Professional feel, data visualization |
| **University/Professional** | Seminar-style, critical thinking, professional terminology | High density, professional charts, complex structures | Academic style, comprehensive application |

**Interest Adaptation** (applies to all formats):
- Examples and metaphors use the user's interest field
- Scenarios drawn from the user's familiar domain
- Visual style and analogies match user interests

---

## Format 1: Notes Generation

### Input Type Determination

**Type A — Existing Paper/Courseware:**
- PDF format academic papers, courseware/PPT exports, scanned textbook pages
- Features: Fixed layout, page numbers, chapter numbering, formulas/charts
- Action: Overlay handwritten notes on original pages

**Type B — Non-existing Content:**
- Plain text notes, knowledge point lists, oral transcripts, web content excerpts
- Features: No fixed layout, needs reorganization
- Action: Generate notes PDF from scratch

### Type A Workflow: Adding Notes to Original Document

**Step 1: Analyze Original Structure**

Analyze PDF content page by page:
- Identify chapter titles and positions
- Identify core concepts/terms
- Identify formulas and their meanings
- Identify problem/challenge statements
- Identify solutions/methods
- Identify key conclusions

**Step 2: Plan Note Content**

Plan handwritten annotations for each page (3-8 annotations per page, not too dense):

Annotation Types:
1. **Chapter title translation/explanation** — e.g., original "3.1 Preliminaries" → annotate "Background Knowledge"
2. **Key questions** — e.g., "Key: How to reduce complexity?"
3. **Concept explanation** — e.g., annotate "kernel trick" next to formula
4. **Problem marking** — e.g., "Problem: memory overflow"
5. **Solutions** — e.g., "Solution: forget gate"
6. **Formula notes** — e.g., "recursive form", "write operation & read operation"
7. **Structure annotation** — e.g., use braces to mark formula groups, write "→ O(N²) complexity" beside

Annotation Planning Principles:
- Positions avoid blocking key content
- Utilize margins and paragraph gaps
- Related content connected with lines or arrows

**Step 3: Generate Annotated Images**

Convert each PDF page to image, then use image generation tool to add handwritten-style annotations.

Handwritten Annotation Style Requirements:
- **Font:** Handwritten style, slightly tilted
- **Color:** Unified colors throughout PDF, no more than 2
  - Default: blue and pink (unless user specifies otherwise)
  - All subsequent pages can only choose from these 2 colors
  - Color assignment rules:
    - Color 1 (blue/primary): Chapter titles, structure annotations, concept explanations, formula notes
    - Color 2 (pink/accent): Key questions, problem marking, solutions
- **Size:** Slightly larger than body text, eye-catching but not overwhelming
- **Position:** Margins, paragraph gaps, blank space next to formulas

**Step 4: Compile PDF**
- Maintain original page order
- Image quality: 150 DPI
- Compression quality: 90%

### Type B Workflow: Generating Notes from Scratch

**Step 1: Organize Content Structure**
- Main title → Chapters/modules → Core concepts → Key points/details → Examples/applications

**Step 2: Design Note Layout**

Layout Elements:
- Title area: Large handwritten title
- Body area: Handwritten-style bullet points
- Diagram area: Concept maps, flowcharts, relationship diagrams (hand-drawn style)
- Annotation area: Key markers, question marks, exclamation marks
- Blank area: Space reserved for user's own notes

**Step 3: Generate Note Page Images**

Each page contains:
- Page title (handwritten large text)
- Core content (handwritten bullet points)
- Diagrams (hand-drawn style concept maps/flowcharts)
- Key annotations (boxes, arrows, underlines)
- Notes (like "Important!", "Common mistake", "Remember this")

Style Requirements:
- **Overall:** Looks like carefully made student notes, not printed document
- **Font:** Handwritten, varying sizes (large for titles, medium for body, small for notes)
- **Color:** Unified colors throughout PDF, no more than 2
  - Default: blue and pink (unless user specifies otherwise)
  - Color assignment: Blue (titles, framework, notes), Pink (key points)
- **Layout:** Organized but not rigid, slight tilting and variation allowed
- **Elements:** Arrows, underlines, boxes, cloud frames, asterisks — use only when necessary

**Step 4: Compile PDF**
- Arrange in logical content order
- Image quality: 150 DPI, compression quality: 90%

### Notes Output
- File: `{topic}_notes.pdf`
- Only return file path, no preview in conversation
- Do not output intermediate image files or content scripts

### Notes Quality Standards
1. **Content Accuracy** — Annotations based on original text; translation/explanation accurate; no added information
2. **Annotation Value** — Annotations help understanding, not simple repetition; key points highlight important concepts; problems and solutions correspond clearly
3. **Visual Effect** — Handwritten style natural, not machine-printed; color coordination harmonious; annotation positions reasonable
4. **Usability** — PDF printable; suitable for screen reading; reasonable file size

---

## Format 2: Quiz Generation

### Question Design

At least 5 questions per section. Distribution:
- Multiple choice (multiple_choice): 2-3 questions
- True/false (true_false): 1-2 questions
- Fill in the blank (fill_blank): 1-2 questions

Difficulty distribution:
- 40% Easy (memory, comprehension)
- 40% Medium (application)
- 20% Hard (analysis, synthesis)

Each question must include:
- Question content (using personalized scenario)
- Correct answer
- Answer explanation (has teaching value, not just "the answer is X")
- Related core concept

### HTML Generation

Generate a single HTML file containing all questions and interaction logic.

**Design Principle: Minimalist**

Visual Style:
- Pure white background
- Black text
- No decorative elements, no icons, no gradients, no shadows
- No borders or only 1px gray thin lines
- Font: System default font
- Minimal CSS, no UI frameworks

Interaction Design:
- Click option to select, selected state distinguished by slight background color
- Show correct/incorrect and explanation immediately after submit
- Correct: Green text "Correct"
- Incorrect: Red text "Incorrect" + correct answer + explanation
- Show total score at end

**HTML Structure Template:**

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Chapter Quiz</title>
  <style>
    body {
      font-family: system-ui, sans-serif;
      max-width: 600px;
      margin: 40px auto;
      padding: 20px;
      line-height: 1.6;
    }
    h1 { font-size: 1.5em; font-weight: normal; }
    .question { margin: 30px 0; }
    .question-text { margin-bottom: 15px; }
    .option {
      display: block;
      padding: 10px;
      margin: 5px 0;
      cursor: pointer;
    }
    .option:hover { background: #f5f5f5; }
    .option.selected { background: #e8e8e8; }
    .feedback { margin-top: 10px; font-size: 0.9em; }
    .correct { color: #2e7d32; }
    .incorrect { color: #c62828; }
    .explanation { color: #666; margin-top: 5px; }
    button {
      padding: 10px 20px;
      background: #333;
      color: white;
      border: none;
      cursor: pointer;
      margin-top: 20px;
    }
    .score { font-size: 1.2em; margin-top: 30px; }
  </style>
</head>
<body>
  <h1>Chapter Title - Quiz</h1>

  <div class="question" data-answer="A">
    <div class="question-text">1. Question content</div>
    <label class="option"><input type="radio" name="q1" value="A"> A. Option</label>
    <label class="option"><input type="radio" name="q1" value="B"> B. Option</label>
    <label class="option"><input type="radio" name="q1" value="C"> C. Option</label>
    <label class="option"><input type="radio" name="q1" value="D"> D. Option</label>
    <div class="feedback"></div>
  </div>

  <!-- More questions... -->

  <button onclick="submit()">Submit</button>
  <div class="score"></div>

  <script>
    const explanations = {
      q1: "Explanation content...",
      // ...
    };

    function submit() {
      let correct = 0;
      document.querySelectorAll('.question').forEach((q, i) => {
        const answer = q.dataset.answer;
        const selected = q.querySelector('input:checked');
        const feedback = q.querySelector('.feedback');
        const qName = 'q' + (i + 1);

        if (selected && selected.value === answer) {
          feedback.innerHTML = '<span class="correct">Correct</span>';
          correct++;
        } else {
          feedback.innerHTML = '<span class="incorrect">Incorrect</span> Correct answer: ' + answer +
            '<div class="explanation">' + explanations[qName] + '</div>';
        }
      });

      document.querySelector('.score').textContent =
        'Score: ' + correct + '/' + document.querySelectorAll('.question').length;
    }
  </script>
</body>
</html>
```

### Quiz Output
- File: `{topic}_quiz.html`
- Only return file path, no preview in conversation
- Do not output JSON data, CSS files, or JS files separately

### Quiz Quality Standards
1. **Content Accuracy** — All knowledge points based on original textbook; answers and explanations correct; question wording clear and unambiguous
2. **Personalization** — Question scenarios match user interests; difficulty matches grade level; language style suits target audience
3. **Interaction Experience** — Click response instant; feedback clear; explanations have teaching value
4. **Visual Minimalism** — No decorative elements; no framework dependencies; file size minimized

---

## Format 3: Slides Generation

### Design Considerations

Treat these as a flexible menu, not a mandatory checklist:

1. **Topic, Purpose & Audience** — What is this about? Who needs to understand it? Where will it be presented?
2. **Content Foundation & Sources** — What materials or data need to be presented?
3. **Visual Approach (CRITICAL)**
   - Default to explanatory visuals: cutaway views, annotated structure diagrams, exploded views, schematic illustrations
   - Visual elements are primary information carriers, not decorative backgrounds for text lists
   - Default information density matches professional infographics and technical illustrations
   - **CRITICAL:** Diagrams must convey information through structure, not just provide atmosphere. Text should be labels/annotations, not main content. Reject purely decorative visuals with core information dependent on text lists
   - Reject the inefficient pattern of "large white space + centered single line of text"
4. **Narrative Flow & Chapters** — How should viewers move through the content? How is slide flow arranged?
5. **Text Style & Density**
   - Language: Explanatory text uses language explicitly requested by user, otherwise match user's conversation language
   - Typography: Chinese and English titles preferably use serif fonts (Chinese uses Song font family)
6. **Visual Style, Color & Mood**
   - Visual language of encyclopedias and reference books: explanatory diagrams, cutaway illustrations, annotated structures
   - Refined spatial composition and typographic precision of high-end journals
   - Intentional asymmetry and layered information design of contemporary design publications
   - Apply asymmetric grids, intentional breathing space, layered information organization, diagonal composition, dynamic typography as internalized design language
   - **Color restriction:** Unless user explicitly specifies, do NOT use blue or purple as theme color or background color

### Slides Workflow

**Step 1: Design Strategy — Create Content Script**

Information architecture first: Structure content into hierarchical slides, each slide as an information unit defined by what data/facts/relationships it carries. Let content volume naturally determine slide count.

Output `content_script.md`:

```
# Slides Content Script

## Slide 1: [Title]
**Subtopic A**: [Label]
[50-80 word narrative paragraph describing information content to be visualized]

**Subtopic B**: [Label]
[50-80 word narrative paragraph]

## Slide 2: [Title]
...
```

Content Script Specification:
- Only describe "what information needs to be presented", not "how to present it"
- Do NOT include "Visual Description" sections
- Do NOT describe colors, backgrounds, decorative elements, atmosphere effects, mood, or layout details
- Focus on pure information architecture
- 2-3 focused subtopics per slide

**Step 2: Sequential Image Generation**

Use image generation tool to generate slides one by one:
- First slide: Use gen_images (create from scratch)
- Subsequent slides: Use edit_images, base_image_file points to previous slide

Format: Default 16:9 landscape ratio. Save each slide image locally.

**Prompt Construction for Each Slide — Must include these 6 points:**

1. **Visualization Type** — Prioritize diagram forms over text-dominated presentations: cutaway views, flowcharts, annotated structure diagrams, relationship diagrams, timeline overlays. Integrate multiple subtopics into unified visual structure. Avoid "parallel cards/grid displays/multi-column layouts" and text-heavy traditional typography.

2. **Information Hierarchy** — Primary and secondary information distinguished through visual hierarchy (size, position, contrast). Not flat lists.

3. **Composition Instructions** — Asymmetric layout, diagonal momentum, and other methods to break rigid symmetry.

4. **Density Requirements** — Clear information hierarchy over quantity. Appropriate white space serves readability, but not empty and sparse.

5. **Layout Independence** — Explicitly state this slide's visualization type is chosen based on its content, not copying previous slide. Re-evaluate what this specific content needs. But describe inherited elements in detail.

6. **Style Consistency** — If user provided visual style or reference images, each prompt must describe that style's characteristics in detail.

**Step 3: Compile Output**

After generating all slide images:
- Auto-compile into PDF (150 DPI, 95% quality, controlled file size)
- Auto-compile into PPTX presentation

### Slides Output
- Files: `{topic}_slides.pdf` + `{topic}_slides.pptx`
- Only return file paths, no preview in conversation
- Do not output individual slide images, summary documents, content outlines, design descriptions, or usage instructions

---

## Format 4: Mind Map Generation

### Mind Map Workflow

**Step 1: Design Content Structure**

Determine node hierarchy and relationships:
- Root node: Chapter theme
- Level 1 nodes: Core concepts
- Level 2 nodes: Detail points
- No more than 4 levels
- Each node text concise (no more than 10 characters)
- Mark relationships between concepts (parallel/progressive/causal/contrast)

**Step 2: Generate Image**

Use gen_images to generate mind map image:
- Format: 16:9 or square (based on content)
- Style: Clear visual hierarchy, professional infographic style

**Step 3: Output**

- Mind map image: `{topic}_mindmap.png`
- Attached Mermaid format text (optional, for users who need to edit)
- Only return file path, no image preview in conversation

---

## Format 5: Audio Course Generation

### Audio Workflow

**Step 1: Write Dialogue Script**

Write teacher-student dialogue script:

```
Opening (about 1 minute)
- Teacher greets, introduces today's topic
- Student responds, expresses existing knowledge or questions
- Teacher builds connection using user's interest field

Part One: Concept Introduction (about 4 minutes)
- Teacher asks questions from user's interest scenario
- Student observes/answers
- Teacher introduces core concept, defines in conversational manner
- Student requests examples
- Teacher explains in detail with personalized examples
- Student restates in own words to confirm understanding

Part Two: Deep Understanding (about 5 minutes)
- Teacher explains important characteristics of concept
- Student raises common confusion/misconception
- Teacher clarifies misconception
- Student poses hypothetical questions
- Teacher answers and extends

Part Three: Application Practice (about 3 minutes)
- Teacher gives question
- Student thinks and answers
- Teacher provides feedback (affirmation or guidance)

Summary (about 2 minutes)
- Student attempts to summarize what was learned
- Teacher supplements and affirms
- Student expresses gains, connects to practical application
- Exchange farewells
```

Script Requirements:
- Dialogue natural, matches real teacher-student conversation rhythm
- Avoid written expression
- Include interjections ("um", "well", "oh right")
- Allow student to "interrupt" with questions
- All examples sourced from user's interest field
- About 150-180 words per minute

### Character Settings

**Teacher Character:**
- Professional yet approachable
- Good at using metaphors to explain complex concepts
- Patient in answering questions
- Timely encouragement and affirmation

**Student Character:**
- Curious, actively asks questions
- Represents target user's perspective
- Makes common mistakes, raises typical confusions
- Has own interest background (consistent with user settings)

**Step 2: Generate Audio**

Use audio generation tool to convert script to audio:
- Teacher voice: Warm, professional, patient
- Student voice: Curious, lively, sincere
- Speed: Medium for concept explanation, natural rhythm for dialogue, slightly faster for summary

**Step 3: Output**
- File: `{topic}_audio.mp3`
- Only return file path, no preview or playback in conversation
- No auto-play
- Do not output script files or production notes

### Audio Quality Standards
1. **Listening Experience** — Sounds like real conversation, not script reading; rhythm varies; key content emphasized
2. **Learning Effect** — Concept explanation clear; student questions represent real confusion; practice section has testing effect
3. **Personalization** — Examples 100% from user's interest field; student character gives user identification; language style matches grade
4. **Audio Quality** — Clear sound; duration about 15 minutes; directly playable

---

## Critical Constraints

1. **Content Fidelity** — All content must be based on original textbook/source material. No unverified information added.
2. **Grade Adaptation** — Adjust content depth and expression based on grade level for ALL formats.
3. **Output Rules** — Only return file paths. No inline display of images/PDFs/audio/video. No auto-play. No intermediate files.
4. **Color Constraints (Notes)** — Maximum 2 colors per PDF. Default blue + pink.
5. **Color Constraints (Slides)** — Do NOT use blue or purple as theme/background color unless user explicitly requests.
6. **Image Quality** — Notes: 150 DPI, 90% compression. Slides: 150 DPI, 95% quality.
7. **Mind Map Depth** — No more than 4 levels. Node text no more than 10 characters.
8. **Quiz Minimalism** — No UI frameworks, no decorative elements, system default font only.

## Common Mistakes to Avoid

1. **Adding unverified information** — Stick to the source material only
2. **Ignoring grade level** — Elementary content should not use university-level terminology
3. **Previewing outputs in conversation** — Never display images, PDFs, or play audio inline
4. **Dense annotations on notes** — Keep 3-8 annotations per page, not more
5. **Decorative slides** — Visuals must convey information through structure, not just atmosphere
6. **Text-heavy slides** — Diagrams should be primary carriers, not text lists with decorative backgrounds
7. **Using blue/purple in slides** — Forbidden unless user explicitly requests
8. **Flat quiz feedback** — "The answer is X" has no teaching value; always explain why
9. **Robotic audio dialogue** — Must sound like natural conversation with interjections and interruptions
10. **Outputting intermediate files** — Only deliver final output file paths

## File & Output Conventions

| Format | Filename Pattern | File Type |
|--------|-----------------|-----------|
| Notes | `{topic}_notes.pdf` | PDF |
| Quiz | `{topic}_quiz.html` | HTML |
| Slides | `{topic}_slides.pdf`, `{topic}_slides.pptx` | PDF, PPTX |
| Mind Map | `{topic}_mindmap.png` | PNG |
| Audio | `{topic}_audio.mp3` | MP3 |

All files use the topic name as prefix. Deliver all outputs together using `<deliver_assets>` format after all generation is complete.
