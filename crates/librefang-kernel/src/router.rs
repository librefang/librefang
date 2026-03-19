use librefang_types::agent::AgentManifest;
use regex_lite::Regex;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

const ROUTING_EXCLUDED_TEMPLATES: &[&str] = &["assistant", "router"];
// Keep this in sync with crates/librefang-cli/src/bundled_agents.rs so the kernel
// can route against bundled templates even before they are written to disk.
const BUNDLED_TEMPLATE_MANIFESTS: &[(&str, &str)] = &[
    (
        "analyst",
        include_str!("../../../agents/analyst/agent.toml"),
    ),
    (
        "architect",
        include_str!("../../../agents/architect/agent.toml"),
    ),
    (
        "assistant",
        include_str!("../../../agents/assistant/agent.toml"),
    ),
    ("coder", include_str!("../../../agents/coder/agent.toml")),
    (
        "code-reviewer",
        include_str!("../../../agents/code-reviewer/agent.toml"),
    ),
    (
        "customer-support",
        include_str!("../../../agents/customer-support/agent.toml"),
    ),
    (
        "data-scientist",
        include_str!("../../../agents/data-scientist/agent.toml"),
    ),
    (
        "debugger",
        include_str!("../../../agents/debugger/agent.toml"),
    ),
    (
        "devops-lead",
        include_str!("../../../agents/devops-lead/agent.toml"),
    ),
    (
        "doc-writer",
        include_str!("../../../agents/doc-writer/agent.toml"),
    ),
    (
        "email-assistant",
        include_str!("../../../agents/email-assistant/agent.toml"),
    ),
    (
        "health-tracker",
        include_str!("../../../agents/health-tracker/agent.toml"),
    ),
    (
        "hello-world",
        include_str!("../../../agents/hello-world/agent.toml"),
    ),
    (
        "home-automation",
        include_str!("../../../agents/home-automation/agent.toml"),
    ),
    (
        "legal-assistant",
        include_str!("../../../agents/legal-assistant/agent.toml"),
    ),
    (
        "meeting-assistant",
        include_str!("../../../agents/meeting-assistant/agent.toml"),
    ),
    ("ops", include_str!("../../../agents/ops/agent.toml")),
    (
        "orchestrator",
        include_str!("../../../agents/orchestrator/agent.toml"),
    ),
    (
        "personal-finance",
        include_str!("../../../agents/personal-finance/agent.toml"),
    ),
    (
        "planner",
        include_str!("../../../agents/planner/agent.toml"),
    ),
    (
        "recruiter",
        include_str!("../../../agents/recruiter/agent.toml"),
    ),
    (
        "recipe-assistant",
        include_str!("../../../agents/recipe-assistant/agent.toml"),
    ),
    (
        "researcher",
        include_str!("../../../agents/researcher/agent.toml"),
    ),
    ("router", include_str!("../../../agents/router/agent.toml")),
    (
        "sales-assistant",
        include_str!("../../../agents/sales-assistant/agent.toml"),
    ),
    (
        "security-auditor",
        include_str!("../../../agents/security-auditor/agent.toml"),
    ),
    (
        "social-media",
        include_str!("../../../agents/social-media/agent.toml"),
    ),
    (
        "test-engineer",
        include_str!("../../../agents/test-engineer/agent.toml"),
    ),
    (
        "translator",
        include_str!("../../../agents/translator/agent.toml"),
    ),
    (
        "travel-planner",
        include_str!("../../../agents/travel-planner/agent.toml"),
    ),
    ("tutor", include_str!("../../../agents/tutor/agent.toml")),
    ("writer", include_str!("../../../agents/writer/agent.toml")),
];
const GENERIC_ENGLISH_WORDS: &[&str] = &[
    "a",
    "agent",
    "an",
    "and",
    "assistant",
    "dedicated",
    "default",
    "expert",
    "for",
    "friendly",
    "general",
    "general-purpose",
    "helper",
    "helpful",
    "management",
    "multi-language",
    "multilingual",
    "of",
    "or",
    "planning",
    "professional",
    "productivity",
    "senior",
    "specialist",
    "support",
    "system",
    "task",
    "template",
    "the",
    "tool",
    "to",
    "with",
    "workflow",
];

struct RouteRule {
    target: &'static str,
    strong: &'static [(&'static str, &'static str)],
    weak: &'static [(&'static str, &'static str)],
}

#[derive(Debug, Clone)]
struct HandRouteCandidate {
    hand_id: String,
    strong_phrases: Vec<String>,
    weak_phrases: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HandSelection {
    pub hand_id: Option<String>,
    pub reason: String,
    pub score: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateSelection {
    pub template: String,
    pub reason: String,
    pub score: usize,
}

#[derive(Debug, Clone)]
struct ManifestRouteCandidate {
    template: String,
    strong_phrases: Vec<String>,
    weak_phrases: Vec<String>,
}

// ── Hand routing: data-driven from HAND.toml ────────────────────────────

/// Cached hand route candidates built from bundled HAND.toml definitions.
/// Invalidated alongside `MANIFEST_CACHE` on hot-reload.
static HAND_ROUTE_CACHE: OnceLock<Mutex<Option<Vec<HandRouteCandidate>>>> = OnceLock::new();

/// Invalidate the hand route cache (call alongside `invalidate_manifest_cache`).
pub fn invalidate_hand_route_cache() {
    if let Some(cache) = HAND_ROUTE_CACHE.get() {
        if let Ok(mut guard) = cache.lock() {
            *guard = None;
        }
    }
}

fn hand_route_candidates() -> Vec<HandRouteCandidate> {
    let cache = HAND_ROUTE_CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(ref cached) = *guard {
        return cached.clone();
    }

    let candidates = build_hand_route_candidates();
    *guard = Some(candidates.clone());
    candidates
}

fn build_hand_route_candidates() -> Vec<HandRouteCandidate> {
    let mut candidates = Vec::new();

    // TODO: also scan ~/.librefang/hands/*/HAND.toml for user-installed hands
    // once hand_route_candidates() receives the home_dir parameter.
    for (id, toml_content, _skill) in librefang_hands::bundled::bundled_hands() {
        let Ok(def) = librefang_hands::bundled::parse_bundled(id, toml_content, "") else {
            continue;
        };

        // Strong: explicit aliases + description-derived phrases
        let mut strong = def.routing.aliases.clone();
        strong.extend(description_phrases(&def.description));

        // Weak: explicit weak_aliases + id-derived tokens
        let mut weak = def.routing.weak_aliases.clone();
        weak.extend(
            def.id
                .to_lowercase()
                .split(['-', '_'])
                .filter(|token| token.len() >= 3 && !GENERIC_ENGLISH_WORDS.contains(token))
                .map(str::to_string),
        );

        candidates.push(HandRouteCandidate {
            hand_id: def.id,
            strong_phrases: dedupe(strong),
            weak_phrases: dedupe(weak),
        });
    }

    candidates
}

const TEMPLATE_RULES: &[RouteRule] = &[
    RouteRule {
        target: "hello-world",
        strong: &[
            ("hello", r"\bhello\b|\bhi\b|\bhey\b|\bgreet\b|\bwelcome\b"),
            ("打招呼", r"打个招呼|欢迎词|自我介绍|介绍你自己|你好"),
        ],
        weak: &[],
    },
    RouteRule {
        target: "coder",
        strong: &[
            (
                "implement",
                r"\bimplement\b|\bbuild\b|\brefactor\b|\bpatch\b",
            ),
            ("写代码", r"写代码|实现功能|补丁|脚本|编码|重构|开发"),
        ],
        weak: &[
            ("code", r"\bcode\b|\bfunction\b|\bapi\b"),
            ("代码", r"代码|程序|模块|接口"),
        ],
    },
    RouteRule {
        target: "debugger",
        strong: &[
            ("debug", r"\bdebug\b|traceback|stack trace"),
            (
                "排查报错",
                r"报错|异常|错误日志|定位根因|排查 bug|故障排查|崩溃",
            ),
        ],
        weak: &[("bug", r"\bbug\b"), ("日志", r"日志|失败原因|根因")],
    },
    RouteRule {
        target: "test-engineer",
        strong: &[
            (
                "test",
                r"\btest(?:s|ing)?\b|unit test|integration test|regression",
            ),
            (
                "测试",
                r"测试用例|单元测试|集成测试|回归测试|覆盖率|验收测试",
            ),
        ],
        weak: &[("验证", r"验证|测试")],
    },
    RouteRule {
        target: "code-reviewer",
        strong: &[
            (
                "code review",
                r"code review|review this diff|review this pr",
            ),
            ("代码审查", r"代码审查|评审这个改动|审查代码|找回归风险"),
        ],
        weak: &[("diff", r"\bdiff\b|\bpr\b|\breview\b")],
    },
    RouteRule {
        target: "architect",
        strong: &[
            (
                "architecture",
                r"architecture|system design|technical design",
            ),
            (
                "架构设计",
                r"架构设计|模块划分|接口设计|技术方案|演进路线|系统设计",
            ),
        ],
        weak: &[("边界", r"边界|拓扑|模块")],
    },
    RouteRule {
        target: "security-auditor",
        strong: &[
            (
                "security",
                r"security|vulnerability|threat model|xss|csrf|sql injection",
            ),
            (
                "安全审计",
                r"安全审计|漏洞|攻击面|鉴权|权限提升|sql注入|合规风险",
            ),
        ],
        weak: &[("安全", r"安全|认证|授权|加密")],
    },
    RouteRule {
        target: "devops-lead",
        strong: &[
            (
                "deploy",
                r"\bdeploy\b|ci/cd|kubernetes|docker|helm|infra|sre",
            ),
            (
                "部署运维",
                r"部署|上线|发布流程|监控告警|容器|k8s|devops|基础设施",
            ),
        ],
        weak: &[("ops", r"运维平台|集群|流水线")],
    },
    RouteRule {
        target: "researcher",
        strong: &[
            ("research", r"\bresearch\b|look up|fact check|latest"),
            ("调研", r"调研|研究|查资料|搜集来源|最新进展|事实核查"),
        ],
        weak: &[("来源", r"来源|资料|背景|现状"), ("对比", r"对比")],
    },
    RouteRule {
        target: "analyst",
        strong: &[
            (
                "analysis",
                r"business analysis|competitive analysis|trend analysis",
            ),
            (
                "业务分析",
                r"竞品分析|趋势分析|报表分析|指标分析|漏斗分析|商业分析",
            ),
        ],
        weak: &[("分析", r"分析|指标|趋势|报表")],
    },
    RouteRule {
        target: "data-scientist",
        strong: &[
            (
                "ml",
                r"machine learning|regression|classification|forecast model|a/?b test",
            ),
            (
                "数据科学",
                r"数据科学|统计建模|回归分析|分类模型|实验设计|预测模型",
            ),
        ],
        weak: &[("统计", r"统计|特征工程|建模")],
    },
    RouteRule {
        target: "planner",
        strong: &[
            ("plan", r"\bplan\b|roadmap|timeline|milestone"),
            (
                "项目计划",
                r"项目计划|里程碑|排期|执行计划|实施计划|三周计划|任务拆解|优先级",
            ),
        ],
        weak: &[("依赖", r"依赖|风险|计划")],
    },
    RouteRule {
        target: "writer",
        strong: &[
            ("writing", r"write an article|blog post|rewrite|polish"),
            ("写作", r"写一篇|起草文章|改写|润色|文案|写作"),
        ],
        weak: &[("内容创作", r"文章|博客|内容创作|内容策划|文章草稿")],
    },
    RouteRule {
        target: "tutor",
        strong: &[
            (
                "teach",
                r"teach me|explain step by step|lesson plan|tutor me",
            ),
            (
                "教学辅导",
                r"教我|讲解这个概念|一步一步解释|辅导学习|辅导功课|作业辅导",
            ),
        ],
        weak: &[("练习", r"练习题|知识讲解|学习计划|教学")],
    },
    RouteRule {
        target: "doc-writer",
        strong: &[
            ("docs", r"\bdocs?\b|readme|api docs|documentation"),
            (
                "技术文档",
                r"技术文档|操作手册|接口文档|教程文档|架构文档|说明文档",
            ),
        ],
        weak: &[("README", r"readme|文档")],
    },
    RouteRule {
        target: "translator",
        strong: &[
            ("translate", r"\btranslate\b|\btranslation\b|localization"),
            ("翻译", r"翻译|本地化|中译英|英译中|日译中|术语统一"),
        ],
        weak: &[("译", r"译成|翻成")],
    },
    RouteRule {
        target: "email-assistant",
        strong: &[
            ("email", r"\bemail\b|\bmail\b|draft reply"),
            ("邮件", r"邮件草稿|回复邮件|收件箱|邮件总结|邮件跟进"),
        ],
        weak: &[("邮箱", r"邮箱|邮件")],
    },
    RouteRule {
        target: "meeting-assistant",
        strong: &[
            ("meeting", r"meeting notes|agenda|action items"),
            ("会议", r"会议纪要|会议议程|行动项|会前准备|会后总结"),
        ],
        weak: &[("纪要", r"会议|纪要|议程")],
    },
    RouteRule {
        target: "social-media",
        strong: &[
            (
                "social",
                r"social media|twitter|linkedin post|content calendar",
            ),
            ("社媒", r"社交媒体|发帖|推文|微博|小红书|内容日历"),
        ],
        weak: &[("帖子", r"帖子|社媒")],
    },
    RouteRule {
        target: "sales-assistant",
        strong: &[
            ("sales", r"sales outreach|crm|pipeline|prospect"),
            ("销售", r"销售跟进|客户开发|销售线索|商机|管道管理|crm"),
        ],
        weak: &[("客户", r"销售|客户跟进")],
    },
    RouteRule {
        target: "customer-support",
        strong: &[
            ("support", r"support ticket|customer support|faq reply"),
            ("客服", r"客服回复|投诉处理|工单|售后|客户支持"),
        ],
        weak: &[("工单", r"客服|工单|售后")],
    },
    RouteRule {
        target: "recruiter",
        strong: &[
            (
                "recruiting",
                r"resume|curriculum vitae|job description|candidate",
            ),
            ("招聘", r"招聘|简历筛选|岗位描述|候选人|面试"),
        ],
        weak: &[("简历", r"简历|面试|岗位")],
    },
    RouteRule {
        target: "legal-assistant",
        strong: &[
            ("legal", r"contract|terms of service|privacy policy|nda"),
            ("法务", r"法律|合同|条款|合规|法务|隐私政策"),
        ],
        weak: &[("协议", r"协议|免责声明")],
    },
    RouteRule {
        target: "personal-finance",
        strong: &[
            ("finance", r"budget|expense|cash flow|asset allocation"),
            ("财务", r"财务规划|预算|支出分析|现金流|储蓄计划|理财"),
        ],
        weak: &[("花销", r"预算|花销|财务")],
    },
    RouteRule {
        target: "recipe-assistant",
        strong: &[
            ("recipe", r"recipe|meal plan|ingredient substitut|portion"),
            ("食谱", r"食谱|菜谱|做菜|烹饪|膳食计划|配料替换"),
        ],
        weak: &[("烹饪", r"菜|做饭|食材|烹饪")],
    },
    RouteRule {
        target: "travel-planner",
        strong: &[
            ("travel", r"travel itinerary|trip plan|flight|hotel"),
            ("旅行", r"行程规划|旅行|旅游计划|机票酒店|路线安排"),
        ],
        weak: &[("出行", r"出行|航班|酒店|景点")],
    },
    RouteRule {
        target: "health-tracker",
        strong: &[
            ("health", r"health tracker|fitness plan|diet log|sleep"),
            ("健康", r"健康记录|健身计划|饮食追踪|睡眠|体重变化"),
        ],
        weak: &[("健康跟踪", r"健康|运动|饮食|睡眠|体重|用药")],
    },
    RouteRule {
        target: "home-automation",
        strong: &[
            ("smart home", r"home assistant|smart home|iot"),
            ("智能家居", r"智能家居|自动化场景|传感器|设备联动"),
        ],
        weak: &[("设备", r"家庭自动化|设备|iot")],
    },
    RouteRule {
        target: "ops",
        strong: &[
            (
                "incident",
                r"incident response|service status|restore service",
            ),
            ("系统运维", r"系统状态|故障恢复|运行诊断|服务异常|恢复服务"),
        ],
        weak: &[("状态", r"状态检查|运行状态|故障")],
    },
    RouteRule {
        target: "orchestrator",
        strong: &[
            (
                "orchestrate",
                r"orchestrate|delegate|multi-agent|multi step",
            ),
            ("复杂协调", r"多代理|多个专家|复杂任务|拆成子任务|协调执行"),
        ],
        weak: &[("编排", r"编排|协作|分工")],
    },
];

/// Minimum score required for a hand match to be considered. A single weak
/// keyword hit (score 1) is too noisy — require at least one strong hit (3)
/// or two weak hits (2) to route to a hand.
const MIN_HAND_SCORE: usize = 2;

/// Select the best hand for a message using keyword matching.
///
/// Keywords are loaded from HAND.toml `[routing]` sections (English-only)
/// and augmented with description-derived phrases. For cross-lingual
/// matching, the caller can provide optional `semantic_scores` computed
/// via embedding cosine similarity against hand descriptions.
pub fn auto_select_hand(
    message: &str,
    semantic_scores: Option<&HashMap<String, f32>>,
) -> HandSelection {
    let mut scored: Vec<(usize, String, Vec<String>)> = Vec::new();

    for candidate in hand_route_candidates() {
        let strong_hits: Vec<String> = candidate
            .strong_phrases
            .iter()
            .filter(|phrase| phrase_matches(message, phrase))
            .cloned()
            .collect();
        let weak_hits: Vec<String> = candidate
            .weak_phrases
            .iter()
            .filter(|phrase| phrase_matches(message, phrase))
            .cloned()
            .collect();
        let mut score = strong_hits.len() * 3 + weak_hits.len();

        // Blend semantic similarity when available (0.0–1.0 → 0–3 bonus points)
        if let Some(scores) = semantic_scores {
            if let Some(&sim) = scores.get(&candidate.hand_id) {
                let bonus = (sim * 3.0).round() as usize;
                score += bonus;
            }
        }

        if score >= MIN_HAND_SCORE {
            let mut hits = strong_hits;
            hits.extend(weak_hits);
            scored.push((score, candidate.hand_id.clone(), hits));
        }
    }

    if scored.is_empty() {
        return HandSelection {
            hand_id: None,
            reason: "no hand match".to_string(),
            score: 0,
        };
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.2.len().cmp(&a.2.len())));
    let (score, hand_id, hits) = scored.remove(0);

    HandSelection {
        hand_id: Some(hand_id.clone()),
        reason: format!("matched {hand_id} via {}", hits.join(", ")),
        score,
    }
}

pub fn auto_select_template(message: &str, agents_dir: &Path) -> TemplateSelection {
    let normalized = message.to_lowercase();
    let metadata_match = auto_select_template_from_metadata(message, agents_dir);
    let mut scored: Vec<(usize, &'static str, Vec<&'static str>)> = Vec::new();

    for rule in TEMPLATE_RULES {
        let strong_hits = matched_labels(message, rule.strong);
        let weak_hits = matched_labels(message, rule.weak);
        let score = strong_hits.len() * 3 + weak_hits.len();
        if score > 0 {
            let mut hits = strong_hits;
            hits.extend(weak_hits);
            scored.push((score, rule.target, hits));
        }
    }

    if scored.is_empty() {
        return metadata_match.unwrap_or_else(|| TemplateSelection {
            template: "orchestrator".to_string(),
            reason: "no direct specialist match; defaulted to orchestrator".to_string(),
            score: 0,
        });
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.2.len().cmp(&a.2.len())));
    let (best_score, best_template, best_hits) = &scored[0];

    if scored.len() > 1 {
        let (second_score, second_template, _) = &scored[1];
        let multi_domain = ["同时", "分别", "协作", "多个", "multi", "together"]
            .iter()
            .any(|token| normalized.contains(token));
        if *second_score > 0 && best_template != second_template && multi_domain {
            return TemplateSelection {
                template: "orchestrator".to_string(),
                reason: format!(
                    "multiple specialties matched ({best_template}, {second_template}); routed to orchestrator"
                ),
                score: *best_score,
            };
        }
    }

    if let Some(metadata_match) = metadata_match {
        if metadata_match.template != *best_template
            && metadata_match.score > *best_score
            && (*best_score <= 1 || metadata_match.score >= *best_score + 2)
        {
            return metadata_match;
        }
    }

    let hits = best_hits.join(", ");
    TemplateSelection {
        template: (*best_template).to_string(),
        reason: if hits.is_empty() {
            format!("matched {best_template}")
        } else {
            format!("matched {best_template} via {hits}")
        },
        score: *best_score,
    }
}

pub fn load_template_manifest(home_dir: &Path, template: &str) -> Result<AgentManifest, String> {
    load_template_manifest_at(&home_dir.join("agents"), template)
}

fn auto_select_template_from_metadata(
    message: &str,
    agents_dir: &Path,
) -> Option<TemplateSelection> {
    let mut scored: Vec<(usize, String, Vec<String>)> = Vec::new();

    for candidate in manifest_route_candidates(agents_dir) {
        let strong_hits: Vec<String> = candidate
            .strong_phrases
            .iter()
            .filter(|phrase| phrase_matches(message, phrase))
            .cloned()
            .collect();
        let weak_hits: Vec<String> = candidate
            .weak_phrases
            .iter()
            .filter(|phrase| phrase_matches(message, phrase))
            .cloned()
            .collect();
        let score = strong_hits.len() * 3 + weak_hits.len();
        if score > 0 {
            let mut hits = strong_hits;
            hits.extend(weak_hits);
            scored.push((score, candidate.template.clone(), hits));
        }
    }

    if scored.is_empty() {
        return None;
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.2.len().cmp(&a.2.len())));
    let (score, template, hits) = scored.remove(0);

    Some(TemplateSelection {
        template: template.clone(),
        reason: format!(
            "matched {template} via manifest metadata: {}",
            hits.join(", ")
        ),
        score,
    })
}

/// Cached manifest route candidates. Invalidated via `invalidate_manifest_cache()`,
/// which should be called on config hot-reload or agent install/uninstall.
static MANIFEST_CACHE: OnceLock<Mutex<Option<Vec<ManifestRouteCandidate>>>> = OnceLock::new();

/// Invalidate the cached manifest route candidates so they are rebuilt on the
/// next routing call. Call this after config hot-reload or agent changes.
pub fn invalidate_manifest_cache() {
    if let Some(cache) = MANIFEST_CACHE.get() {
        if let Ok(mut guard) = cache.lock() {
            *guard = None;
        }
    }
}

fn manifest_route_candidates(agents_dir: &Path) -> Vec<ManifestRouteCandidate> {
    let cache = MANIFEST_CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(ref cached) = *guard {
        return cached.clone();
    }

    let candidates = build_manifest_route_candidates(agents_dir);
    *guard = Some(candidates.clone());
    candidates
}

fn build_manifest_route_candidates(agents_dir: &Path) -> Vec<ManifestRouteCandidate> {
    let mut candidates = Vec::new();

    for template in all_template_names(agents_dir) {
        if ROUTING_EXCLUDED_TEMPLATES.contains(&template.as_str()) {
            continue;
        }

        let Ok(manifest) = load_template_manifest_at(agents_dir, &template) else {
            continue;
        };
        let (routing_aliases, routing_weak_aliases, exclude_generated) =
            manifest_routing_config(&manifest);

        let generated_strong = {
            let mut phrases = english_variants(&template);
            phrases.extend(tag_phrases(&manifest.tags));
            phrases.extend(description_phrases(&manifest.description));
            phrases
        };

        let strong_source = if exclude_generated {
            routing_aliases
        } else {
            let mut values = routing_aliases;
            values.extend(generated_strong);
            values
        };

        let mut weak_source = routing_weak_aliases;
        weak_source.extend(
            template
                .to_lowercase()
                .split(['-', '_'])
                .filter(|token| token.len() >= 3 && !GENERIC_ENGLISH_WORDS.contains(token))
                .map(str::to_string),
        );

        candidates.push(ManifestRouteCandidate {
            template,
            strong_phrases: dedupe(strong_source),
            weak_phrases: dedupe(weak_source),
        });
    }

    candidates
}

fn load_template_manifest_at(agents_dir: &Path, template: &str) -> Result<AgentManifest, String> {
    let manifest_path = agents_dir.join(template).join("agent.toml");
    if manifest_path.exists() {
        let manifest_toml = fs::read_to_string(&manifest_path)
            .map_err(|e| format!("failed to read {}: {e}", manifest_path.display()))?;
        return toml::from_str::<AgentManifest>(&manifest_toml)
            .map_err(|e| format!("failed to parse {}: {e}", manifest_path.display()));
    }

    if let Some(manifest_toml) = bundled_template_manifest(template) {
        return toml::from_str::<AgentManifest>(manifest_toml)
            .map_err(|e| format!("failed to parse bundled manifest '{template}': {e}"));
    }

    Err(format!(
        "template '{template}' not found in {} or bundled manifests",
        agents_dir.display()
    ))
}

fn template_names_from_dir(root: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };

    let mut names = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && path.join("agent.toml").exists() {
            if let Some(name) = path.file_name().and_then(|value| value.to_str()) {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    names
}

fn all_template_names(agents_dir: &Path) -> Vec<String> {
    let mut names: HashSet<String> = template_names_from_dir(agents_dir).into_iter().collect();
    names.extend(
        BUNDLED_TEMPLATE_MANIFESTS
            .iter()
            .map(|(name, _)| (*name).to_string()),
    );

    let mut ordered: Vec<String> = names.into_iter().collect();
    ordered.sort();
    ordered
}

fn bundled_template_manifest(template: &str) -> Option<&'static str> {
    BUNDLED_TEMPLATE_MANIFESTS
        .iter()
        .find(|(name, _)| *name == template)
        .map(|(_, manifest)| *manifest)
}

fn matched_labels(message: &str, patterns: &[(&'static str, &'static str)]) -> Vec<&'static str> {
    patterns
        .iter()
        .filter_map(|(label, pattern)| regex_matches(message, pattern).then_some(*label))
        .collect()
}

/// Global cache of compiled regex patterns (keyed by the raw pattern string).
/// Avoids recompiling the same patterns on every incoming message.
static REGEX_CACHE: OnceLock<Mutex<HashMap<String, Regex>>> = OnceLock::new();

fn regex_matches(message: &str, pattern: &str) -> bool {
    let cache = REGEX_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = cache.lock().unwrap_or_else(|e| e.into_inner());
    let regex = map.entry(pattern.to_string()).or_insert_with(|| {
        match Regex::new(&format!("(?i){pattern}")) {
            Ok(r) => r,
            Err(_) => Regex::new("(?!x)x").unwrap(), // never-match sentinel
        }
    });
    regex.is_match(message)
}

fn english_variants(text: &str) -> Vec<String> {
    let normalized = text.trim().to_lowercase();
    if normalized.is_empty() {
        return Vec::new();
    }

    let mut variants = vec![normalized.clone()];
    if normalized.contains('-') || normalized.contains('_') {
        variants.push(normalized.replace(['-', '_'], " "));
        variants.extend(
            normalized
                .split(['-', '_'])
                .filter(|part| part.len() >= 3)
                .map(str::to_string),
        );
    }
    dedupe(variants)
}

fn description_phrases(description: &str) -> Vec<String> {
    let text = description.trim();
    if text.is_empty() {
        return Vec::new();
    }

    let mut phrases = Vec::new();

    for chunk in split_phrase_chunks(text) {
        if chunk.is_empty() {
            continue;
        }

        if is_ascii_phrase(&chunk) {
            phrases.extend(ascii_phrase_candidates(&chunk, 4));
        } else if is_meaningful_unicode_phrase(&chunk) {
            phrases.push(chunk);
        }
    }

    dedupe(phrases)
}

fn tag_phrases(tags: &[String]) -> Vec<String> {
    let mut phrases = Vec::new();

    for tag in tags {
        let normalized = tag.trim();
        if normalized.is_empty() {
            continue;
        }

        if is_ascii_phrase(normalized) {
            phrases.extend(ascii_phrase_candidates(normalized, 3));
        } else if is_meaningful_unicode_phrase(normalized) {
            phrases.push(normalized.to_string());
        }
    }

    dedupe(phrases)
}

fn manifest_routing_config(manifest: &AgentManifest) -> (Vec<String>, Vec<String>, bool) {
    let Some(Value::Object(routing)) = manifest.metadata.get("routing") else {
        return (Vec::new(), Vec::new(), false);
    };

    let mut aliases = json_string_list(routing.get("aliases"));
    aliases.extend(json_string_list(routing.get("strong_aliases")));

    (
        aliases,
        json_string_list(routing.get("weak_aliases")),
        routing
            .get("exclude_generated")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    )
}

fn json_string_list(value: Option<&Value>) -> Vec<String> {
    let Some(Value::Array(items)) = value else {
        return Vec::new();
    };

    items
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn phrase_matches(message: &str, phrase: &str) -> bool {
    let candidate = phrase.trim();
    if candidate.is_empty() {
        return false;
    }

    if is_ascii_phrase(candidate) {
        let escaped = regex_lite::escape(&candidate.to_lowercase()).replace("\\ ", r"[\s_-]+");
        let pattern = format!(r"(?i)(^|[^a-z0-9]){}([^a-z0-9]|$)", escaped);
        return Regex::new(&pattern)
            .map(|regex| regex.is_match(&message.to_lowercase()))
            .unwrap_or(false);
    }

    message.to_lowercase().contains(&candidate.to_lowercase())
}

fn split_phrase_chunks(text: &str) -> Vec<String> {
    text.split(is_phrase_separator)
        .filter_map(normalize_phrase_chunk)
        .collect()
}

fn is_phrase_separator(ch: char) -> bool {
    ch == '\n'
        || ch == '\r'
        || ch == '\t'
        || (ch.is_ascii_punctuation() && !matches!(ch, '-' | '_'))
        || matches!(
            ch,
            '\u{3001}' // 、
                | '\u{3002}' // 。
                | '\u{FF0C}' // ，
                | '\u{FF1B}' // ；
                | '\u{FF1A}' // ：
                | '\u{FF08}' // （
                | '\u{FF09}' // ）
                | '\u{2013}' // –
                | '\u{2014}' // —
        )
}

fn normalize_phrase_chunk(raw: &str) -> Option<String> {
    let trimmed = raw.trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '_' && ch != '-');
    if trimmed.is_empty() {
        return None;
    }

    if !is_ascii_phrase(trimmed) {
        return Some(trimmed.to_string());
    }

    let words: Vec<&str> = trimmed
        .split([' ', '-', '_'])
        .filter(|word| !word.is_empty())
        .collect();
    let start = words
        .iter()
        .position(|word| !GENERIC_ENGLISH_WORDS.contains(&word.to_ascii_lowercase().as_str()))
        .unwrap_or(words.len());
    let end = words
        .iter()
        .rposition(|word| !GENERIC_ENGLISH_WORDS.contains(&word.to_ascii_lowercase().as_str()))
        .map(|idx| idx + 1)
        .unwrap_or(0);

    if start >= end {
        return None;
    }

    Some(words[start..end].join(" "))
}

fn ascii_phrase_candidates(text: &str, min_len: usize) -> Vec<String> {
    let normalized = text.trim().to_lowercase();
    if normalized.is_empty() {
        return Vec::new();
    }

    let content_words: Vec<String> = normalized
        .split([' ', '-', '_'])
        .filter(|word| word.len() >= min_len && !GENERIC_ENGLISH_WORDS.contains(word))
        .map(str::to_string)
        .collect();
    let mut phrases = Vec::new();

    if normalized.len() >= min_len
        && normalized.split_whitespace().count() <= 4
        && normalized
            .split_whitespace()
            .any(|word| !GENERIC_ENGLISH_WORDS.contains(&word))
    {
        phrases.extend(english_variants(&normalized));
    }

    phrases.extend(content_words.iter().cloned());
    for window in content_words.windows(2) {
        phrases.push(window.join(" "));
    }

    dedupe(phrases)
}

fn is_meaningful_unicode_phrase(text: &str) -> bool {
    (2..=32).contains(&text.chars().count())
}

fn is_ascii_phrase(value: &str) -> bool {
    value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ' ' | '_' | '-'))
}

fn dedupe(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut ordered = Vec::new();

    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_string()) {
            ordered.push(trimmed.to_string());
        }
    }

    ordered
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LibreFangKernel;
    use librefang_types::config::KernelConfig;
    use std::process::Command;
    use tempfile::tempdir;

    /// Helper: call auto_select_hand without semantic scores.
    fn hand(msg: &str) -> HandSelection {
        auto_select_hand(msg, None)
    }

    #[test]
    fn test_auto_select_hand_prefers_browser_tasks() {
        let selection = hand("open website and navigate to the login page");
        assert_eq!(selection.hand_id, Some("browser".to_string()));
        assert!(selection.score > 0);
    }

    #[test]
    fn test_auto_select_template_prefers_explicit_coder_rule() {
        // Invalidate cache to ensure clean state for subsequent tests
        invalidate_manifest_cache();

        let selection = auto_select_template(
            "请实现一个新的 Rust API 并补丁修复它",
            Path::new("/tmp/does-not-exist"),
        );
        assert_eq!(selection.template, "coder");
        assert!(selection.score > 0);
    }

    #[test]
    fn test_auto_select_template_can_use_manifest_metadata() {
        // Invalidate manifest cache and force rebuild to ensure fresh scan
        invalidate_manifest_cache();

        let tmp = tempdir().unwrap();
        let agents_dir = tmp.path().join("agents");
        let template_dir = agents_dir.join("release-notes");
        fs::create_dir_all(&template_dir).unwrap();
        fs::write(
            template_dir.join("agent.toml"),
            r#"
name = "release-notes"
description = "Drafts release notes and changelogs."
module = "builtin:chat"
tags = ["release-notes", "changelog"]

[model]
provider = "default"
model = "default"
system_prompt = "unused"

[metadata.routing]
aliases = ["release notes"]
weak_aliases = ["changelog"]
"#,
        )
        .unwrap();

        let selection =
            auto_select_template("Please draft release notes for version 1.2.3", &agents_dir);
        assert_eq!(selection.template, "release-notes");
        assert!(selection.score > 0);
    }

    #[test]
    fn test_auto_select_template_routes_multi_domain_to_orchestrator() {
        // Invalidate cache to ensure clean state for subsequent tests
        invalidate_manifest_cache();

        let selection = auto_select_template(
            "请同时写代码并做深度调研，然后协作输出方案",
            Path::new("/tmp/does-not-exist"),
        );
        assert_eq!(selection.template, "orchestrator");
        assert!(selection.score > 0);
    }

    #[test]
    fn test_description_phrases_extract_language_agnostic_keywords() {
        let phrases = description_phrases(
            "Friendly multi-language translation agent for document translation, localization, and cross-cultural communication.",
        );
        assert!(phrases.contains(&"translation".to_string()));
        assert!(phrases.contains(&"document".to_string()));
        assert!(phrases.contains(&"localization".to_string()));
        assert!(phrases.contains(&"cross cultural".to_string()));
        assert!(!phrases.contains(&"friendly".to_string()));
    }

    #[test]
    fn test_tag_phrases_keep_non_ascii_tags_without_language_specific_rules() {
        let phrases = tag_phrases(&["分析".to_string(), "release-notes".to_string()]);
        assert!(phrases.contains(&"分析".to_string()));
        assert!(phrases.contains(&"release notes".to_string()));
    }

    #[test]
    fn test_bundled_template_metadata_routes_common_intents() {
        let cases = [
            (
                "Perform a threat model and vulnerability review for this service",
                "security-auditor",
            ),
            ("Draft a reply email to this customer", "email-assistant"),
            (
                "Create a travel itinerary for Kyoto this weekend",
                "travel-planner",
            ),
            ("Translate this product page into Japanese", "translator"),
            ("Write a test plan for this release", "test-engineer"),
            (
                "Prepare meeting notes and action items",
                "meeting-assistant",
            ),
            (
                "Help me design a system architecture for this service",
                "architect",
            ),
            (
                "Break this project into milestones and dependencies",
                "planner",
            ),
            ("Investigate this bug and find the root cause", "debugger"),
            (
                "Do deep web research and gather sources on this topic",
                "researcher",
            ),
        ];

        for (message, expected) in cases {
            let selection = auto_select_template(message, Path::new("/tmp/does-not-exist"));
            assert_eq!(selection.template, expected, "message: {message}");
            assert!(selection.score > 0, "message: {message}");
        }
    }

    // ── Hand routing: all hands coverage (English keywords from HAND.toml) ──

    #[test]
    fn test_auto_select_hand_routes_collector() {
        let sel = hand("please monitor changes on this repo and track updates");
        assert_eq!(sel.hand_id, Some("collector".to_string()));
    }

    #[test]
    fn test_auto_select_hand_routes_researcher() {
        let sel = hand("do a deep research and systematic review of the landscape");
        assert_eq!(sel.hand_id, Some("researcher".to_string()));
    }

    #[test]
    fn test_auto_select_hand_routes_clip() {
        let sel = hand("clip video and do subtitle extraction");
        assert_eq!(sel.hand_id, Some("clip".to_string()));
    }

    #[test]
    fn test_auto_select_hand_routes_predictor() {
        let sel = hand("predict the probability and forecast this outcome");
        assert_eq!(sel.hand_id, Some("predictor".to_string()));
    }

    #[test]
    fn test_auto_select_hand_routes_trader() {
        let sel = hand("check my portfolio and do market analysis");
        assert_eq!(sel.hand_id, Some("trader".to_string()));
    }

    #[test]
    fn test_auto_select_hand_routes_lead() {
        let sel = hand("do lead generation and build a prospect list");
        assert_eq!(sel.hand_id, Some("lead".to_string()));
    }

    #[test]
    fn test_auto_select_hand_routes_analytics() {
        let sel = hand("run data analysis and create a dashboard report");
        assert_eq!(sel.hand_id, Some("analytics".to_string()));
    }

    #[test]
    fn test_auto_select_hand_routes_apitester() {
        let sel = hand("run an api test and endpoint test on this service");
        assert_eq!(sel.hand_id, Some("apitester".to_string()));
    }

    #[test]
    fn test_auto_select_hand_routes_devops() {
        let sel = hand("set up ci/cd pipeline and infrastructure monitoring");
        assert_eq!(sel.hand_id, Some("devops".to_string()));
    }

    #[test]
    fn test_auto_select_hand_routes_strategist() {
        let sel = hand("do a strategic analysis and competitive analysis");
        assert_eq!(sel.hand_id, Some("strategist".to_string()));
    }

    #[test]
    fn test_auto_select_hand_routes_linkedin() {
        let sel = hand("optimize my linkedin profile optimization strategy");
        assert_eq!(sel.hand_id, Some("linkedin".to_string()));
    }

    #[test]
    fn test_auto_select_hand_routes_reddit() {
        let sel = hand("post on reddit subreddit and monitor replies");
        assert_eq!(sel.hand_id, Some("reddit".to_string()));
    }

    #[test]
    fn test_auto_select_hand_routes_twitter() {
        let sel = hand("post a tweet on twitter with scheduled tweet");
        assert_eq!(sel.hand_id, Some("twitter".to_string()));
    }

    // ── MIN_HAND_SCORE threshold ────────────────────────────────────

    #[test]
    fn test_weak_only_single_match_rejected() {
        // Single weak keyword should be below MIN_HAND_SCORE=2
        let sel = hand("help me deploy");
        assert_eq!(sel.hand_id, None, "single weak match should be rejected");
        assert_eq!(sel.score, 0);
    }

    #[test]
    fn test_strong_match_always_passes_threshold() {
        // A single strong match = score 3 >= MIN_HAND_SCORE(2)
        let sel = hand("open website please");
        assert_eq!(sel.hand_id, Some("browser".to_string()));
        assert!(sel.score >= 2);
    }

    // ── Negative / boundary tests ───────────────────────────────────

    #[test]
    fn test_generic_greeting_no_hand_match() {
        let sel = hand("hello, how is the weather today?");
        assert_eq!(sel.hand_id, None);
    }

    #[test]
    fn test_generic_english_no_hand_match() {
        let sel = hand("Hello, how are you today?");
        assert_eq!(sel.hand_id, None);
    }

    #[test]
    fn test_coding_request_no_hand_match() {
        let sel = hand("please write a Rust function for me");
        assert_eq!(sel.hand_id, None);
    }

    #[test]
    fn test_ambiguous_short_message_no_hand_match() {
        let sel = hand("help me look at this");
        assert_eq!(sel.hand_id, None);
    }

    // ── Scoring: strong beats weak ──────────────────────────────────

    #[test]
    fn test_strong_match_scores_higher_than_weak() {
        // "deep research" is a strong alias for researcher (score 3+)
        let strong = hand("do deep research on this topic");
        // "research" alone is weak for researcher (score 1, rejected)
        let weak = hand("research this");
        assert!(strong.score > weak.score);
    }

    #[test]
    fn test_multiple_strong_matches_boost_score() {
        let single = hand("run data analysis");
        let double = hand("run data analysis and build a dashboard with automated report");
        assert!(double.score >= single.score);
    }

    // ── Semantic score blending ─────────────────────────────────────

    #[test]
    fn test_semantic_scores_boost_hand_selection() {
        // Without semantic: generic message should not match any hand
        let without = hand("please help me with this task");
        assert_eq!(without.hand_id, None);

        // With semantic: simulated high similarity to "collector"
        let mut scores = HashMap::new();
        scores.insert("collector".to_string(), 0.9);
        let with = auto_select_hand("please help me with this task", Some(&scores));
        assert_eq!(with.hand_id, Some("collector".to_string()));
        assert!(with.score >= MIN_HAND_SCORE);
    }

    #[test]
    fn test_semantic_fallback_routes_chinese_to_collector() {
        // Chinese input: no English keyword match → score 0 without semantic
        let without = hand("帮我监控这个网站的变更");
        assert_eq!(
            without.hand_id, None,
            "Chinese should not match English keywords"
        );

        // With embedding similarity: "帮我监控这个网站的变更" would be semantically
        // close to collector's description "monitors any target continuously with
        // change detection". Simulated here with a high cosine score.
        let mut scores = HashMap::new();
        scores.insert("collector".to_string(), 0.85);
        scores.insert("browser".to_string(), 0.3);
        let with = auto_select_hand("帮我监控这个网站的变更", Some(&scores));
        assert_eq!(with.hand_id, Some("collector".to_string()));
    }

    #[test]
    fn test_semantic_fallback_routes_japanese_to_trader() {
        // Japanese: "株式取引のポートフォリオを確認して" (check stock trading portfolio)
        let without = hand("株式取引のポートフォリオを確認して");
        assert_eq!(without.hand_id, None);

        let mut scores = HashMap::new();
        scores.insert("trader".to_string(), 0.82);
        scores.insert("analytics".to_string(), 0.25);
        let with = auto_select_hand("株式取引のポートフォリオを確認して", Some(&scores));
        assert_eq!(with.hand_id, Some("trader".to_string()));
    }

    #[test]
    fn test_semantic_fallback_routes_korean_to_researcher() {
        // Korean: "이 주제에 대해 심층 연구를 해주세요" (do deep research on this topic)
        let mut scores = HashMap::new();
        scores.insert("researcher".to_string(), 0.88);
        let with = auto_select_hand("이 주제에 대해 심층 연구를 해주세요", Some(&scores));
        assert_eq!(with.hand_id, Some("researcher".to_string()));
    }

    #[test]
    fn test_semantic_low_similarity_does_not_match() {
        // All scores below threshold: similarity too low to trigger routing
        let mut scores = HashMap::new();
        scores.insert("collector".to_string(), 0.2);
        scores.insert("browser".to_string(), 0.15);
        scores.insert("trader".to_string(), 0.1);
        let sel = auto_select_hand("一些随便的话", Some(&scores));
        // 0.2 * 3 = 0.6, rounds to 1 — below MIN_HAND_SCORE(2)
        assert_eq!(sel.hand_id, None, "low similarity should not match");
    }

    #[test]
    fn test_semantic_plus_keyword_combined_scoring() {
        // English keyword gives partial score, semantic boosts it over threshold
        // "deploy" is a weak alias for devops (score 1, below threshold alone)
        let keyword_only = hand("help me deploy the service");
        // May or may not match depending on whether deploy hits weak alias
        let keyword_score = keyword_only.score;

        // With semantic boost: devops similarity adds bonus points
        let mut scores = HashMap::new();
        scores.insert("devops".to_string(), 0.75);
        let combined = auto_select_hand("help me deploy the service", Some(&scores));
        assert!(
            combined.score > keyword_score,
            "semantic should boost keyword score"
        );
        assert_eq!(combined.hand_id, Some("devops".to_string()));
    }

    #[test]
    fn test_no_embedding_graceful_degradation() {
        // When semantic_scores is None, only keyword matching is used.
        // Non-English input simply gets no match (graceful, not error).
        let sel = auto_select_hand("请帮我做数据分析", None);
        assert_eq!(sel.hand_id, None, "should gracefully return no match");
        assert_eq!(sel.score, 0);
    }

    #[test]
    fn test_semantic_does_not_override_strong_keyword() {
        // If keyword matching strongly matches hand A, but semantic scores
        // favor hand B, keyword should still win (keyword score is higher).
        let mut scores = HashMap::new();
        scores.insert("trader".to_string(), 0.9); // semantic favors trader
                                                  // But message strongly matches browser via keywords
        let sel = auto_select_hand("open website and navigate to the login page", Some(&scores));
        // Browser should win because keyword score (3+) > trader semantic (2-3)
        assert_eq!(sel.hand_id, Some("browser".to_string()));
    }

    // ── Cache consistency ───────────────────────────────────────────

    #[test]
    fn test_hand_route_cache_returns_consistent_results() {
        let r1 = hand("open website and fill form");
        let r2 = hand("open website and fill form");
        assert_eq!(r1.hand_id, r2.hand_id);
        assert_eq!(r1.score, r2.score);
    }

    #[test]
    fn test_load_template_manifest_falls_back_to_bundled_template() {
        let tmp = tempdir().unwrap();
        let manifest = load_template_manifest(tmp.path(), "assistant").unwrap();
        assert_eq!(manifest.name, "assistant");
    }

    #[test]
    fn test_load_template_manifest_prefers_local_override() {
        let tmp = tempdir().unwrap();
        let template_dir = tmp.path().join("agents").join("assistant");
        fs::create_dir_all(&template_dir).unwrap();
        fs::write(
            template_dir.join("agent.toml"),
            r#"
name = "assistant"
description = "Local override"
module = "builtin:chat"

[model]
provider = "default"
model = "default"
system_prompt = "override"
"#,
        )
        .unwrap();

        let manifest = load_template_manifest(tmp.path(), "assistant").unwrap();
        assert_eq!(manifest.description, "Local override");
    }

    #[tokio::test]
    async fn test_builtin_router_spawns_metadata_template_and_cleans_up() {
        // Skip if Python is not available
        if Command::new("python3").arg("--version").output().is_err()
            && Command::new("python").arg("--version").output().is_err()
        {
            eprintln!("Python not available, skipping router execution test");
            return;
        }

        // Skip if no LLM API key is available OR if the default model is not accessible
        // This test requires a working LLM configuration
        let has_anthropic = std::env::var("ANTHROPIC_API_KEY").is_ok();
        let has_groq = std::env::var("GROQ_API_KEY").is_ok();
        let has_openai = std::env::var("OPENAI_API_KEY").is_ok();

        if !has_anthropic && !has_groq && !has_openai {
            eprintln!("No LLM API key available, skipping router execution test");
            return;
        }

        // If we have API keys, try to boot the kernel and see if it works
        // Skip if the default model is not accessible
        let tmp = tempdir().unwrap();
        let home_dir = tmp.path();
        let agents_dir = home_dir.join("agents");
        let template_dir = agents_dir.join("release-notes");
        fs::create_dir_all(&template_dir).unwrap();
        fs::write(
            home_dir.join("router_worker.py"),
            r#"#!/usr/bin/env python3
import json
import sys

payload = json.loads(sys.stdin.readline())
print(json.dumps({"type": "response", "text": f"ROUTED:{payload['message']}"}))
"#,
        )
        .unwrap();
        fs::write(
            template_dir.join("agent.toml"),
            r#"
name = "release-notes"
description = "Drafts release notes and changelogs."
module = "python:router_worker.py"
tags = ["release notes", "changelog"]

[model]
provider = "default"
model = "default"
system_prompt = "unused"

[metadata.routing]
aliases = ["release notes"]
weak_aliases = ["changelog"]
"#,
        )
        .unwrap();

        let config = KernelConfig {
            home_dir: home_dir.to_path_buf(),
            data_dir: home_dir.join("data"),
            ..KernelConfig::default()
        };
        let kernel = match LibreFangKernel::boot_with_config(config) {
            Ok(k) => k,
            Err(e) => {
                // Skip if kernel fails to boot (e.g., no valid LLM driver)
                eprintln!("Kernel boot failed: {e}, skipping router execution test");
                return;
            }
        };

        let router_id = kernel
            .registry
            .find_by_name("router")
            .map(|entry| entry.id)
            .expect("default router should exist");

        let result = match kernel
            .send_message(router_id, "Please draft release notes for version 1.2.3")
            .await
        {
            Ok(r) => r,
            Err(e) => {
                // Skip if LLM call fails (e.g., model not accessible)
                eprintln!("LLM call failed: {e}, skipping router execution test");
                kernel.shutdown();
                return;
            }
        };

        assert_eq!(
            result.response,
            "ROUTED:Please draft release notes for version 1.2.3"
        );
        assert!(kernel.registry.find_by_name("release-notes").is_none());

        kernel.shutdown();
    }
}
