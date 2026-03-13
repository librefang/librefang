use librefang_types::agent::AgentManifest;
use regex_lite::Regex;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandSelection {
    pub hand_id: Option<&'static str>,
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

const HAND_RULES: &[RouteRule] = &[
    RouteRule {
        target: "collector",
        strong: &[
            ("monitor", r"监控|watch|跟踪更新|追踪发布|持续观察|订阅变化"),
            (
                "collect",
                r"收集公开信息|情报收集|公开信息采集|osint|抓取变更",
            ),
        ],
        weak: &[("signal", r"变化检测|变更|signal|signals")],
    },
    RouteRule {
        target: "researcher",
        strong: &[
            (
                "deep research",
                r"深度调研|深入研究|systematic review|landscape analysis",
            ),
            ("compare", r"对比.*方案|对比.*框架|全面对比|研究报告"),
        ],
        weak: &[("research", r"调研|研究")],
    },
    RouteRule {
        target: "browser",
        strong: &[
            (
                "browser",
                r"打开网站|网页登录|点击|表单|下单|browser|navigate",
            ),
            ("interactive", r"站内操作|网页操作|填写.*表单|需要登录"),
        ],
        weak: &[],
    },
    RouteRule {
        target: "clip",
        strong: &[
            ("video", r"视频切片|转录视频|下载视频|字幕提取|clip video"),
            ("clip", r"剪辑|短视频|分段视频"),
        ],
        weak: &[],
    },
    RouteRule {
        target: "predictor",
        strong: &[
            ("predict", r"预测|概率判断|胜率|likelihood|forecast this"),
            ("scenario", r"情景推演|概率校准|趋势预测"),
        ],
        weak: &[],
    },
    RouteRule {
        target: "trader",
        strong: &[
            (
                "trade",
                r"交易|下单|仓位|portfolio|期权|股票操作|paper trade",
            ),
            ("market", r"盘前分析|盘后复盘|市场信号|技术面|NVDA|TSLA|BTC"),
        ],
        weak: &[("finance-market", r"美股|加密货币|行情")],
    },
    RouteRule {
        target: "lead",
        strong: &[
            (
                "lead",
                r"线索挖掘|潜在客户|lead gen|prospect list|联系人富化",
            ),
            ("sales-search", r"找.*客户|找.*公司名单|联系人整理"),
        ],
        weak: &[],
    },
];

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

pub fn auto_select_hand(message: &str) -> HandSelection {
    let mut scored: Vec<(usize, &'static str, Vec<&'static str>)> = Vec::new();

    for rule in HAND_RULES {
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
        return HandSelection {
            hand_id: None,
            reason: "no hand match".to_string(),
            score: 0,
        };
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.2.len().cmp(&a.2.len())));
    let (score, hand_id, hits) = &scored[0];

    HandSelection {
        hand_id: Some(hand_id),
        reason: format!("matched {hand_id} via {}", hits.join(", ")),
        score: *score,
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

fn manifest_route_candidates(agents_dir: &Path) -> Vec<ManifestRouteCandidate> {
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

fn regex_matches(message: &str, pattern: &str) -> bool {
    Regex::new(&format!("(?i){pattern}"))
        .map(|regex| regex.is_match(message))
        .unwrap_or(false)
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
        || ch.is_ascii_punctuation()
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
            .any(|word| !GENERIC_ENGLISH_WORDS.contains(word))
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
    use librefang_types::agent::AgentManifest;
    use librefang_types::config::KernelConfig;
    use std::process::Command;
    use tempfile::tempdir;

    #[test]
    fn test_auto_select_hand_prefers_browser_tasks() {
        let selection = auto_select_hand("请打开网站并填写表单提交这个请求");
        assert_eq!(selection.hand_id, Some("browser"));
        assert!(selection.score > 0);
    }

    #[test]
    fn test_auto_select_template_prefers_explicit_coder_rule() {
        let selection = auto_select_template(
            "请实现一个新的 Rust API 并补丁修复它",
            Path::new("/tmp/does-not-exist"),
        );
        assert_eq!(selection.template, "coder");
        assert!(selection.score > 0);
    }

    #[test]
    fn test_auto_select_template_can_use_manifest_metadata() {
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
        if Command::new("python3").arg("--version").output().is_err()
            && Command::new("python").arg("--version").output().is_err()
        {
            eprintln!("Python not available, skipping router execution test");
            return;
        }

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
        let kernel = LibreFangKernel::boot_with_config(config).unwrap();

        let router_manifest: AgentManifest = toml::from_str(
            r#"
name = "router"
description = "Native router"
module = "builtin:router"

[model]
provider = "default"
model = "default"
system_prompt = "unused"
"#,
        )
        .unwrap();
        let router_id = kernel.spawn_agent(router_manifest).unwrap();

        let result = kernel
            .send_message(router_id, "Please draft release notes for version 1.2.3")
            .await
            .unwrap();

        assert_eq!(
            result.response,
            "ROUTED:Please draft release notes for version 1.2.3"
        );
        assert!(kernel.registry.find_by_name("release-notes").is_none());

        kernel.shutdown();
    }
}
