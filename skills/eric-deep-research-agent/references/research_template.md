# DeepResearch Agent - Reference Documentation

## Research Plan Template

```json
{
  "primary_topic": "テーマ名",
  "research_objective": "研究目的の詳細な説明",
  "sub_topics": [
    {
      "name": "サブトピック名",
      "queries": ["検索クエリ1", "検索クエリ2"],
      "priority": 1-5,
      "target_sources": 20
    }
  ],
  "target_sources": 100,
  "timeline_phases": ["Phase 1: Initial Research", "Phase 2: Deep Dive", "Phase 3: Verification"]
}
```

## Source Classification Framework

### Source Types

| Type | Japanese | Priority | Reliability Score |
|------|----------|----------|-------------------|
| Official Documents | 公式文書 | High | 5/5 |
| Academic Papers | 学術論文 | High | 5/5 |
| Government Reports | 政府報告書 | High | 5/5 |
| Industry Whitepapers | 業界白書 | High | 4/5 |
| Established News | 主要新聞・メディア | Medium | 4/5 |
| Company Reports | 企業レポート | Medium | 3/5 |
| Technical Blogs | 技術ブログ | Medium | 3/5 |
| Forums/Communities | フォーラム・コミュニティ | Low | 2/5 |

### Domain Authority Assessment

- **Authoritative**: 政府機関、学術機関、主要企業公式
- **Reliable**: established media, 業界リーダー企業
- **Moderate**: 専門ブログ、有名人ジャーナリスト
- **Caution**: 匿名投稿、更新日古參情報

## Search Query Templates

### Historical Context
```
"[topic] history"
"[topic] evolution"
"[topic] 歴史 背景"
"[topic] 発展 過程"
```

### Technical Specifications
```
"[topic] technical details"
"[topic] technology specifications"
"[topic] 技術 仕様"
"[topic] アーキテクチャ"
```

### Market Analysis
```
"[topic] market size"
"[topic] industry trends"
"[topic] 市場規模"
"[topic] 市場動向 分析"
```

### Challenges & Risks
```
"[topic] challenges"
"[topic] problems issues"
"[topic] 課題 課題点"
"[topic] リスク"
```

### Future Outlook
```
"[topic] future predictions"
"[topic] outlook forecast"
"[topic] 将来展望"
"[topic] 予測"
```

## Content Extraction Matrix

```json
{
  "source_id": "unique_identifier",
  "url": "https://...",
  "title": "Article Title",
  "source_type": "news|academic|whitepaper|blog|forum",
  "date_published": "YYYY-MM-DD",
  "author": "Author Name",
  "language": "ja|en|other",
  "relevance_score": 1-5,
  "key_findings": [
    {
      "finding": "finding description",
      "evidence_type": "statistic|quote|fact|analysis",
      "verification_status": "verified|unverified|contradicted"
    }
  ],
  "contradictions": [
    {
      "issue": "contradicting claim",
      "sources": ["source_id_1", "source_id_2"]
    }
  ],
  "confidence_level": "high|medium|low",
  "notes": "additional observations"
}
```

## Report Citation Format

### Inline Citation
```
Statistic or fact [1]
Quote or specific claim [2]
Analysis or interpretation [3-5]
```

### Reference Entry Format
```
[1] Author(s). "Title." Publication/Website. Date. URL.

Example:
[1] Sato, Y. "AI Technology Trends 2024." Tech Journal. 2024-03-15. https://example.com/article
```

## Quality Checklist

### Pre-Report
- [ ] Minimum 100 unique sources extracted
- [ ] All statistics have 3+ source verification
- [ ] Cross-domain coverage achieved
- [ ] Recent sources prioritized (within 2 years)
- [ ] Contradictions documented

### Report Structure
- [ ] Executive summary present
- [ ] All claims have citations
- [ ] Source diversity visible
- [ ] Tables and figures properly labeled
- [ ] Conclusion clearly states findings

### Verification
- [ ] All links functional
- [ ] Quotes verified against originals
- [ ] Statistics match cited sources
- [ ] No logical fallacies in synthesis
