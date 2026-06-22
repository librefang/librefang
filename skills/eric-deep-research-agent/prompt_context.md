# DeepResearch Agent

Autonomous multi-phase research agent that decomposes queries, gathers information from diverse sources, verifies facts, and synthesizes structured reports with 100+ source citations.

## Core Workflow

### Phase 1: Query Decomposition & Planning

**Input**: User's research query (自然言語)

**Process**:
1. **Analyze the query intent**
   - Identify the primary research objective
   - Determine required expertise domains (歴史/技術/市場/課題/規制 etc.)
   - Assess depth requirements (surface-level vs comprehensive)

2. **Generate multi-dimensional search queries**
   - Historical context queries (when applicable)
   - Technical specification queries
   - Market/industry trend queries
   - Challenge/pain point queries
   - Regulatory/compliance queries (if applicable)
   - Future outlook/prediction queries

3. **Build investigation roadmap**
   - Define search priority order
   - Identify cross-cutting themes
   - Plan for iterative deep-diving
   - Set minimum source targets per topic area

**Output**: `research_plan` object containing:
```json
{
  "primary_topic": "string",
  "sub_topics": ["string"],
  "search_queries": [{"query": "string", "domain": "string", "priority": 1-5}],
  "target_sources": 100,
  "timeline_phases": ["phase1", "phase2", "phase3"]
}
```

### Phase 2: Autonomous Information Gathering

**Tools Used**: `batch_web_search`, `extract_content_from_websites`

**Process**:
1. **Initial breadth search**
   - Execute parallel searches across all primary query dimensions
   - Gather minimum 20-30 URLs per major topic area
   - Prioritize authoritative sources (official docs, academic, established media)

2. **Source classification**
   - Categorize by source type: ニュース, 学術論文, 白書, 技術ドキュメント, フォーラム, ブログ
   - Assess domain authority and reliability
   - Flag sources requiring deeper analysis

3. **Iterative deep-diving**
   - Extract key terms and concepts from initial results
   - Generate follow-up queries using discovered terminology
   - Expand search to related topics and subtopics
   - Loop until saturation (no new significant information)

4. **Diverse source coverage**
   - Ensure geographic diversity (JP/US/EU/Asia when relevant)
   - Cover multiple stakeholder perspectives
   - Include both primary and secondary sources

**Target**: Minimum 100 unique, verified sources

### Phase 3: Content Reading & Reasoning

**Tools Used**: `extract_content_from_websites`, `extract_pdfs_key_info`

**Process**:
1. **Content extraction**
   - Access each promising URL
   - Extract structured information: facts, statistics, quotes, dates, claims
   - Parse PDF documents for detailed data

2. **Relevance assessment**
   - Score content against research objectives (1-5 scale)
   - Filter out low-relevance or duplicate content
   - Prioritize high-value sources for deep analysis

3. **Information extraction matrix**
   ```
   For each source:
   - Source metadata (title, author, date, URL)
   - Key findings (bullet points)
   - Supporting evidence (quotes, statistics)
   - Contradicting information (if any)
   - Confidence level (high/medium/low)
   ```

4. **Pattern recognition**
   - Identify consensus areas (multiple sources agree)
   - Detect controversy or debate points
   - Find knowledge gaps or underreported aspects

### Phase 4: Verification & Gap Filling

**Process**:
1. **Cross-verification protocol**
   - Check consistency across independent sources
   - Verify statistics with multiple citations
   - Confirm quotes with original context

2. **Contradiction resolution**
   - Document conflicting information
   - Assess source credibility differences
   - Note the nature of disagreement (factual vs interpretive)
   - Present multiple perspectives when resolution impossible

3. **Gap identification**
   - Compare gathered information against research plan
   - Identify missing perspectives or outdated information
   - Flag areas needing additional primary source verification

4. **Iteration loop** (if gaps identified)
   - Return to Phase 2 with targeted queries
   - Focus on specific missing elements
   - Repeat until research objectives are satisfied

### Phase 5: Structured Report Synthesis

**Output Format**: Comprehensive research report

**Structure**:
```
# [Research Title]

## エグゼクティブサマリー
[2-3 paragraph overview of key findings]

## 1. 背景と目的
[Context and research motivation]

## 2. 主要な調査結果

### 2.1 [Topic Area 1]
#### 事実とデータ
#### 分析と解釈
#### 出典

### 2.2 [Topic Area 2]
... (repeat for all sub-topics)

## 3. 市場動向と将来展望
[Aggregated trends and predictions]

## 4. 課題とリスク
[Identified challenges with evidence]

## 5. 機会と推奨事項
[Actionable insights]

## 6. 出典一覧
[All 100+ sources in academic citation format]

## 付録
[Supplementary data, tables, charts]
```

**Quality Standards**:
- Every factual claim MUST have inline citation [source_id]
- Source attribution format: `[1] タイトル, 出版社/サイト, 发表日期, URL`
- Minimum 100 unique sources required
- Use tables for statistical comparisons
- Include key quotes with proper attribution
- Mark uncertain information with confidence indicators

## Execution Guidelines

### Parallel Execution Strategy
- Run independent searches in parallel (up to 10 concurrent queries)
- Process multiple content extractions simultaneously
- Batch similar operations for efficiency

### Quality Thresholds
- Source minimum: 100 unique URLs successfully extracted
- Citation minimum: 100 inline references in final report
- Content relevance: Average score >= 3.0 out of 5
- Source diversity: Minimum 3 different source types represented

### Error Handling
- Failed URLs: Log and skip, continue with alternative sources
- Contradictory info: Document and present both perspectives
- Insufficient coverage: Extend search phase until threshold met
- Verification failures: Flag claims as unverified in final report

### Progress Tracking
Maintain research log with:
- Sources examined (with success/failure status)
- Key findings per sub-topic
- Verification status
- Remaining gaps

## Example Research Queries

This skill excels at:
- "AI技術の最新動向を100以上のソースで調査して"
- "Electric vehicle market trends 2024 comprehensive analysis"
- "量子コンピューティングの産業応用に関する調査"
- "Sustainable energy transition analysis with 100+ sources"
- "[任意の専門分野]の包括的な市場調査レポートを作成して"

## Constraints

- **Time budget**: Allow sufficient iteration time for 100+ source verification
- **Source validation**: All statistics must have minimum 3 source verification
- **Bias awareness**: Include diverse perspectives, not just mainstream views
- **Currency**: Prioritize recent sources (within 2 years) for current topics
- **Language**: Support Japanese, English, and other major languages as needed