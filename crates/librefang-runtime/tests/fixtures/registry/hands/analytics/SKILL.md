---
name: analytics-hand-skill
version: "1.0.0"
description: "Expert knowledge for AI data analytics -- statistical methods, visualization best practices, pandas reference, and reporting patterns"
runtime: prompt_only
---

# Data Analytics Expert Knowledge

## pandas Quick Reference

### Data Loading
```python
import pandas as pd

# CSV
df = pd.read_csv('data.csv')
df = pd.read_csv('data.csv', parse_dates=['date_col'], index_col='id')

# JSON
df = pd.read_json('data.json')
df = pd.read_json('data.json', orient='records')

# Excel
df = pd.read_excel('data.xlsx', sheet_name='Sheet1')

# From dict
df = pd.DataFrame({'col1': [1, 2, 3], 'col2': ['a', 'b', 'c']})
```

### Data Inspection
```python
df.shape              # (rows, columns)
df.dtypes             # Column types
df.info()             # Summary including memory usage
df.describe()         # Statistical summary
df.head(10)           # First 10 rows
df.isnull().sum()     # Missing values per column
df.duplicated().sum() # Number of duplicate rows
df.nunique()          # Unique values per column
```

### Data Cleaning
```python
# Handle missing values
df.dropna()                          # Drop rows with any NaN
df.fillna(0)                         # Fill NaN with 0
df.fillna(df.mean())                 # Fill with column means
df['col'].interpolate()              # Interpolate missing values

# Remove duplicates
df.drop_duplicates()
df.drop_duplicates(subset=['col1', 'col2'])

# Type conversion
df['col'] = df['col'].astype(int)
df['date'] = pd.to_datetime(df['date'])
df['cat'] = df['cat'].astype('category')

# Outlier removal (IQR method)
Q1 = df['col'].quantile(0.25)
Q3 = df['col'].quantile(0.75)
IQR = Q3 - Q1
df = df[(df['col'] >= Q1 - 1.5*IQR) & (df['col'] <= Q3 + 1.5*IQR)]
```

### Aggregation & Grouping
```python
# Group by
df.groupby('category').agg({'value': ['mean', 'sum', 'count']})

# Pivot table
pd.pivot_table(df, values='value', index='row_cat', columns='col_cat', aggfunc='mean')

# Cross tabulation
pd.crosstab(df['cat1'], df['cat2'])

# Rolling statistics
df['rolling_mean'] = df['value'].rolling(window=7).mean()

# Percentage change
df['pct_change'] = df['value'].pct_change()
```

### Time Series
```python
# Set datetime index
df.set_index('date', inplace=True)

# Resample
df.resample('W').mean()   # Weekly average
df.resample('M').sum()    # Monthly sum
df.resample('Q').count()  # Quarterly count

# Date range
pd.date_range(start='2025-01-01', periods=30, freq='D')

# Shift/Lag
df['prev_value'] = df['value'].shift(1)
df['next_value'] = df['value'].shift(-1)
```

---

## Visualization Best Practices

### matplotlib + seaborn Reference

```python
import matplotlib
matplotlib.use('Agg')  # Non-interactive backend
import matplotlib.pyplot as plt
import seaborn as sns

# Set style
sns.set_theme(style='whitegrid')
plt.rcParams['figure.figsize'] = (10, 6)
```

### Chart Selection Guide

| Data Type | Question | Chart Type |
|-----------|----------|------------|
| Categorical | Comparison | Bar chart |
| Categorical | Proportion | Pie chart (if <6 categories) |
| Numerical | Distribution | Histogram / Box plot |
| Two numerical | Relationship | Scatter plot |
| Time series | Trend | Line chart |
| Matrix | Correlation | Heatmap |
| Categories + values | Comparison | Grouped bar / Stacked bar |
| Geographical | Location | Map / Choropleth |

### Chart Templates

**Bar Chart**:
```python
fig, ax = plt.subplots(figsize=(10, 6))
data = df['category'].value_counts()
data.plot(kind='bar', ax=ax, color='steelblue')
ax.set_title('Distribution by Category', fontsize=14, fontweight='bold')
ax.set_xlabel('Category')
ax.set_ylabel('Count')
plt.xticks(rotation=45, ha='right')
plt.tight_layout()
plt.savefig('bar_chart.png', dpi=150, bbox_inches='tight')
plt.close()
```

**Line Chart (Time Series)**:
```python
fig, ax = plt.subplots(figsize=(12, 6))
ax.plot(df.index, df['value'], linewidth=2, color='steelblue')
ax.fill_between(df.index, df['value'], alpha=0.1, color='steelblue')
ax.set_title('Trend Over Time', fontsize=14, fontweight='bold')
ax.set_xlabel('Date')
ax.set_ylabel('Value')
plt.tight_layout()
plt.savefig('line_chart.png', dpi=150, bbox_inches='tight')
plt.close()
```

**Correlation Heatmap**:
```python
fig, ax = plt.subplots(figsize=(10, 8))
corr = df.select_dtypes(include='number').corr()
sns.heatmap(corr, annot=True, fmt='.2f', cmap='RdBu_r', center=0, ax=ax)
ax.set_title('Correlation Matrix', fontsize=14, fontweight='bold')
plt.tight_layout()
plt.savefig('heatmap.png', dpi=150, bbox_inches='tight')
plt.close()
```

**Scatter Plot**:
```python
fig, ax = plt.subplots(figsize=(10, 6))
ax.scatter(df['x'], df['y'], alpha=0.6, edgecolors='black', linewidth=0.5)
ax.set_title('X vs Y', fontsize=14, fontweight='bold')
ax.set_xlabel('X Variable')
ax.set_ylabel('Y Variable')
plt.tight_layout()
plt.savefig('scatter.png', dpi=150, bbox_inches='tight')
plt.close()
```

### Visualization Do's and Don'ts

**Do**:
- Start y-axis at 0 for bar charts
- Use consistent colors across related charts
- Label axes clearly with units
- Add titles that describe the insight, not just the data
- Use appropriate scales (log scale for exponential data)

**Don't**:
- Use 3D charts (distorts perception)
- Use more than 6-7 colors in one chart
- Truncate axes to exaggerate differences
- Use pie charts for more than 5 categories
- Add unnecessary chart junk (borders, backgrounds, grids)

---

## Statistical Methods

### Descriptive Statistics
| Measure | pandas | Purpose |
|---------|--------|---------|
| Mean | `df['col'].mean()` | Central tendency |
| Median | `df['col'].median()` | Robust central tendency |
| Std Dev | `df['col'].std()` | Variability |
| Skewness | `df['col'].skew()` | Distribution symmetry |
| Kurtosis | `df['col'].kurtosis()` | Distribution tails |
| Percentiles | `df['col'].quantile([0.25, 0.5, 0.75])` | Distribution spread |

### Correlation Analysis
```python
# Pearson correlation (linear)
df['col1'].corr(df['col2'])

# Spearman correlation (monotonic)
df['col1'].corr(df['col2'], method='spearman')

# Full correlation matrix
df.select_dtypes(include='number').corr()
```

Interpretation:
- |r| > 0.7: Strong correlation
- 0.4 < |r| < 0.7: Moderate correlation
- |r| < 0.4: Weak correlation
- Correlation != Causation

### Hypothesis Testing (scipy)
```python
from scipy import stats

# T-test (compare two group means)
t_stat, p_value = stats.ttest_ind(group1, group2)

# Chi-squared test (categorical independence)
chi2, p_value, dof, expected = stats.chi2_contingency(contingency_table)

# Significance: p < 0.05 is commonly used threshold

# Mann-Whitney U test (non-parametric alternative to t-test)
u_stat, p_value = stats.mannwhitneyu(group1, group2, alternative='two-sided')

# One-way ANOVA (compare 3+ group means)
f_stat, p_value = stats.f_oneway(group1, group2, group3)

# Normality check (determines which test to use)
shapiro_stat, p_value = stats.shapiro(data)  # p > 0.05 means normal
```

### Statistical Significance Decision Guide

**Test selection flowchart:**
| Data Situation | Normal Distribution? | Test to Use |
|---------------|---------------------|-------------|
| Compare 2 group means | Yes | Independent t-test (`ttest_ind`) |
| Compare 2 group means | No | Mann-Whitney U (`mannwhitneyu`) |
| Compare 3+ group means | Yes | One-way ANOVA (`f_oneway`) |
| Compare 3+ group means | No | Kruskal-Wallis (`kruskal`) |
| Compare paired samples | Yes | Paired t-test (`ttest_rel`) |
| Compare paired samples | No | Wilcoxon signed-rank (`wilcoxon`) |
| Test categorical independence | N/A | Chi-squared (`chi2_contingency`) |
| Test correlation | Yes | Pearson (`pearsonr`) |
| Test correlation | No | Spearman (`spearmanr`) |

**P-value interpretation:**
| p-value | Interpretation | Action |
|---------|---------------|--------|
| p < 0.01 | Strong evidence against null hypothesis | Report as statistically significant |
| 0.01 ≤ p < 0.05 | Moderate evidence | Report as significant with caveat |
| 0.05 ≤ p < 0.10 | Weak evidence | Report as marginally significant |
| p ≥ 0.10 | Insufficient evidence | Do not claim significance |

**Practical significance — always report effect size:**
```python
# Cohen's d for comparing two means
def cohens_d(group1, group2):
    n1, n2 = len(group1), len(group2)
    var1, var2 = group1.var(), group2.var()
    pooled_std = ((n1 - 1) * var1 + (n2 - 1) * var2) / (n1 + n2 - 2)
    return (group1.mean() - group2.mean()) / (pooled_std ** 0.5)

# Interpretation: |d| < 0.2 = negligible, 0.2-0.5 = small, 0.5-0.8 = medium, > 0.8 = large
```

**Sample size awareness:**
- n < 30: Use non-parametric tests; results are exploratory
- 30 ≤ n < 100: Parametric tests OK if normality holds; moderate confidence
- n ≥ 100: Central Limit Theorem applies; high confidence in parametric tests
- Always report sample size alongside p-values

**Confidence threshold mapping:**
| Setting | p-value threshold | Minimum effect size | Minimum sample size |
|---------|------------------|--------------------|--------------------|
| High | p < 0.01 | Cohen's d ≥ 0.5 | n ≥ 100 |
| Medium | p < 0.05 | Cohen's d ≥ 0.3 | n ≥ 30 |
| Low | p < 0.10 | Any | Any |

---

## Report Structure Best Practices

### CRISP-DM Framework
1. **Business Understanding**: What question are we answering?
2. **Data Understanding**: What data do we have? Quality?
3. **Data Preparation**: Cleaning, transformation, feature engineering
4. **Modeling**: Statistical analysis, ML models
5. **Evaluation**: Are results valid and useful?
6. **Deployment**: Reports, dashboards, recommendations

### Insight Hierarchy
```
Level 1: What happened (descriptive)
  "Revenue increased 15% last quarter"

Level 2: Why it happened (diagnostic)
  "Revenue increase driven by 30% growth in enterprise segment"

Level 3: What will happen (predictive)
  "Based on current trends, Q2 revenue projected at $X"

Level 4: What to do (prescriptive)
  "Invest in enterprise sales team to capitalize on growth trajectory"
```

### Data Quality Assessment Template
```
| Dimension | Score | Details |
|-----------|-------|---------|
| Completeness | 85% | 15% missing values in 'email' column |
| Accuracy | High | Validated against source system |
| Consistency | Medium | Date formats vary across sources |
| Timeliness | Current | Data refreshed daily |
| Uniqueness | 99% | 1% duplicate records found |
```

---

## Worked Examples

### Example 1: E-commerce Sales Analysis

**Goal**: Analyze 12 months of order data to identify revenue drivers, customer segments, and growth trends.

#### Step 1 — Load and clean
```python
import pandas as pd
import numpy as np

df = pd.read_csv('orders.csv', parse_dates=['order_date'])

# Quick audit
print(f"Rows: {len(df):,}  Columns: {df.shape[1]}")
print(df.isnull().sum()[df.isnull().sum() > 0])

# Clean
df = df.dropna(subset=['customer_id', 'order_total'])
df['order_total'] = df['order_total'].clip(lower=0)  # Remove negative values
df['order_month'] = df['order_date'].dt.to_period('M')
```

#### Step 2 — Revenue trend analysis
```python
monthly = (
    df.groupby('order_month')
    .agg(revenue=('order_total', 'sum'),
         orders=('order_id', 'nunique'),
         customers=('customer_id', 'nunique'))
    .reset_index()
)
monthly['aov'] = monthly['revenue'] / monthly['orders']  # Average order value
monthly['revenue_mom'] = monthly['revenue'].pct_change()  # Month-over-month growth

fig, axes = plt.subplots(2, 1, figsize=(12, 8), sharex=True)
axes[0].bar(monthly['order_month'].astype(str), monthly['revenue'], color='steelblue')
axes[0].set_title('Monthly Revenue', fontsize=14, fontweight='bold')
axes[0].set_ylabel('Revenue ($)')

axes[1].plot(monthly['order_month'].astype(str), monthly['aov'], marker='o', color='coral')
axes[1].set_title('Average Order Value', fontsize=14, fontweight='bold')
axes[1].set_ylabel('AOV ($)')
plt.xticks(rotation=45, ha='right')
plt.tight_layout()
plt.savefig('revenue_trend.png', dpi=150, bbox_inches='tight')
plt.close()
```

#### Step 3 — Customer segmentation (RFM)
```python
snapshot_date = df['order_date'].max() + pd.Timedelta(days=1)

rfm = df.groupby('customer_id').agg(
    recency=('order_date', lambda x: (snapshot_date - x.max()).days),
    frequency=('order_id', 'nunique'),
    monetary=('order_total', 'sum')
)

# Score each dimension 1-4 using quartiles
for col in ['recency', 'frequency', 'monetary']:
    labels = [4, 3, 2, 1] if col == 'recency' else [1, 2, 3, 4]
    rfm[f'{col}_score'] = pd.qcut(rfm[col], q=4, labels=labels, duplicates='drop')

rfm['rfm_score'] = (rfm['recency_score'].astype(int)
                     + rfm['frequency_score'].astype(int)
                     + rfm['monetary_score'].astype(int))

# Segment mapping
def segment(row):
    r, f, m = int(row['recency_score']), int(row['frequency_score']), int(row['monetary_score'])
    if r >= 3 and f >= 3:
        return 'Champions'
    elif r >= 3 and f < 3:
        return 'New / Promising'
    elif r < 3 and f >= 3:
        return 'At Risk'
    else:
        return 'Needs Attention'

rfm['segment'] = rfm.apply(segment, axis=1)
print(rfm.groupby('segment').agg(
    count=('monetary', 'size'),
    avg_revenue=('monetary', 'mean'),
    avg_frequency=('frequency', 'mean')
).sort_values('avg_revenue', ascending=False))
```

#### Step 4 — Cohort retention analysis
```python
df['cohort'] = df.groupby('customer_id')['order_date'].transform('min').dt.to_period('M')
df['order_period'] = df['order_date'].dt.to_period('M')
df['cohort_index'] = (df['order_period'] - df['cohort']).apply(lambda x: x.n)

cohort_table = (
    df.groupby(['cohort', 'cohort_index'])['customer_id']
    .nunique()
    .reset_index()
    .pivot(index='cohort', columns='cohort_index', values='customer_id')
)

# Convert to retention percentages
retention = cohort_table.div(cohort_table[0], axis=0) * 100

fig, ax = plt.subplots(figsize=(14, 8))
sns.heatmap(retention, annot=True, fmt='.0f', cmap='YlOrRd_r', ax=ax)
ax.set_title('Cohort Retention (% of original customers)', fontsize=14, fontweight='bold')
ax.set_xlabel('Months Since First Purchase')
ax.set_ylabel('Cohort')
plt.tight_layout()
plt.savefig('cohort_retention.png', dpi=150, bbox_inches='tight')
plt.close()
```

---

### Example 2: A/B Test Analysis

**Goal**: Evaluate whether a new checkout flow (variant B) improves conversion rate over the existing flow (variant A).

#### Step 1 — Sample size calculation (pre-test)
```python
from scipy import stats
import numpy as np

baseline_rate = 0.12      # Current conversion rate: 12%
mde = 0.02                # Minimum detectable effect: 2 percentage points
alpha = 0.05              # Significance level
power = 0.80              # Statistical power

# Using the normal approximation formula
p1 = baseline_rate
p2 = baseline_rate + mde
p_avg = (p1 + p2) / 2

z_alpha = stats.norm.ppf(1 - alpha / 2)  # Two-tailed
z_beta = stats.norm.ppf(power)

n_per_group = ((z_alpha * np.sqrt(2 * p_avg * (1 - p_avg))
                + z_beta * np.sqrt(p1 * (1 - p1) + p2 * (1 - p2))) ** 2
               / (p2 - p1) ** 2)

print(f"Required sample size per group: {int(np.ceil(n_per_group)):,}")
print(f"Total required: {int(np.ceil(n_per_group)) * 2:,}")
```

#### Step 2 — Run the test and collect results
```python
ab = pd.read_csv('ab_test_results.csv')

summary = ab.groupby('variant').agg(
    visitors=('user_id', 'nunique'),
    conversions=('converted', 'sum')
)
summary['conversion_rate'] = summary['conversions'] / summary['visitors']
print(summary)
```

#### Step 3 — Statistical significance
```python
a = ab[ab['variant'] == 'A']
b = ab[ab['variant'] == 'B']

# Chi-squared test for proportions
contingency = pd.crosstab(ab['variant'], ab['converted'])
chi2, p_value, dof, expected = stats.chi2_contingency(contingency)

# Proportions z-test (more direct)
from statsmodels.stats.proportion import proportions_ztest
successes = [summary.loc['B', 'conversions'], summary.loc['A', 'conversions']]
trials = [summary.loc['B', 'visitors'], summary.loc['A', 'visitors']]
z_stat, p_val = proportions_ztest(successes, trials, alternative='larger')

print(f"Z-statistic: {z_stat:.4f}")
print(f"P-value:     {p_val:.4f}")
print(f"Significant: {'Yes' if p_val < 0.05 else 'No'} (at alpha=0.05)")
```

#### Step 4 — Effect size and confidence interval
```python
p_a = summary.loc['A', 'conversion_rate']
p_b = summary.loc['B', 'conversion_rate']
n_a = summary.loc['A', 'visitors']
n_b = summary.loc['B', 'visitors']

lift = (p_b - p_a) / p_a
se_diff = np.sqrt(p_a * (1 - p_a) / n_a + p_b * (1 - p_b) / n_b)
ci_lower = (p_b - p_a) - 1.96 * se_diff
ci_upper = (p_b - p_a) + 1.96 * se_diff

print(f"Control rate:        {p_a:.4f}")
print(f"Variant rate:        {p_b:.4f}")
print(f"Absolute lift:       {p_b - p_a:+.4f}")
print(f"Relative lift:       {lift:+.2%}")
print(f"95% CI for diff:     [{ci_lower:+.4f}, {ci_upper:+.4f}]")
```

#### Step 5 — Recommendation template
```
## A/B Test Report: New Checkout Flow

| Metric              | Control (A) | Variant (B) |
|---------------------|-------------|-------------|
| Visitors            | 15,204      | 15,198      |
| Conversions         | 1,824       | 2,127       |
| Conversion Rate     | 12.00%      | 13.99%      |

**Result**: Statistically significant (p = 0.0003, alpha = 0.05)
**Lift**: +1.99pp absolute / +16.6% relative
**95% CI**: [+0.90pp, +3.08pp]
**Recommendation**: Deploy variant B. The effect is both statistically
and practically significant with a lower bound above the +1pp threshold.
```

---

### Example 3: Customer Churn Analysis

**Goal**: Identify which factors most strongly predict customer churn and quantify their relative importance.

#### Step 1 — Feature engineering
```python
df = pd.read_csv('customers.csv')

# Create behavioral features from raw data
features = df.copy()
features['tenure_months'] = (pd.Timestamp.now() - pd.to_datetime(df['signup_date'])).dt.days / 30
features['support_tickets_per_month'] = df['total_tickets'] / features['tenure_months'].clip(lower=1)
features['avg_session_minutes'] = df['total_session_minutes'] / df['total_sessions'].clip(lower=1)
features['days_since_last_login'] = (pd.Timestamp.now() - pd.to_datetime(df['last_login'])).dt.days
features['has_premium'] = (df['plan'] == 'premium').astype(int)

# Drop raw columns, keep engineered features
feature_cols = [
    'tenure_months', 'support_tickets_per_month', 'avg_session_minutes',
    'days_since_last_login', 'has_premium', 'monthly_spend', 'num_features_used'
]
```

#### Step 2 — Correlation analysis
```python
churn_corr = features[feature_cols + ['churned']].corr()['churned'].drop('churned').sort_values()

fig, ax = plt.subplots(figsize=(8, 5))
churn_corr.plot(kind='barh', ax=ax, color=['coral' if x > 0 else 'steelblue' for x in churn_corr])
ax.set_title('Feature Correlation with Churn', fontsize=14, fontweight='bold')
ax.set_xlabel('Pearson Correlation')
ax.axvline(x=0, color='black', linewidth=0.5)
plt.tight_layout()
plt.savefig('churn_correlations.png', dpi=150, bbox_inches='tight')
plt.close()
```

#### Step 3 — Key driver identification via group comparison
```python
churned = features[features['churned'] == 1]
retained = features[features['churned'] == 0]

comparison = []
for col in feature_cols:
    t_stat, p_val = stats.ttest_ind(churned[col].dropna(), retained[col].dropna())
    d = cohens_d(churned[col].dropna(), retained[col].dropna())  # From earlier definition
    comparison.append({
        'feature': col,
        'churned_mean': churned[col].mean(),
        'retained_mean': retained[col].mean(),
        'diff_pct': (churned[col].mean() - retained[col].mean()) / retained[col].mean() * 100,
        'cohens_d': abs(d),
        'p_value': p_val,
        'significant': p_val < 0.05
    })

result = pd.DataFrame(comparison).sort_values('cohens_d', ascending=False)
print(result.to_string(index=False))
```

#### Step 4 — Interpret and report
```
## Churn Driver Analysis

**Top 3 factors distinguishing churned vs. retained customers:**

| Factor                     | Churned (avg) | Retained (avg) | Diff     | Effect Size |
|----------------------------|---------------|----------------|----------|-------------|
| Days since last login      | 34.2          | 8.7            | +293%    | Large       |
| Support tickets per month  | 2.8           | 0.9            | +211%    | Large       |
| Number of features used    | 3.1           | 7.4            | -58%     | Medium      |

**Actionable insights:**
1. Customers inactive >14 days are 4x more likely to churn -- trigger re-engagement email at day 10
2. High support ticket rate signals frustration -- escalate accounts with >2 tickets/month to success team
3. Low feature adoption correlates with churn -- implement onboarding flow targeting unused features
```

---

## Advanced pandas Patterns

### Window Functions

```python
# Expanding window (cumulative statistics)
df['cumulative_avg'] = df['value'].expanding().mean()
df['cumulative_max'] = df['value'].expanding().max()

# Exponentially weighted moving average (EWMA) -- emphasizes recent values
df['ewma_7'] = df['value'].ewm(span=7).mean()     # Span-based decay
df['ewma_a'] = df['value'].ewm(alpha=0.3).mean()   # Explicit decay factor

# Comparison: rolling vs. EWMA
# - rolling(7).mean() weights all 7 values equally
# - ewm(span=7).mean() weights recent values exponentially more
# Use EWMA when recent data matters more (stock prices, real-time metrics)

# Rolling with min_periods (handles early rows with insufficient data)
df['rolling_avg'] = df['value'].rolling(window=30, min_periods=5).mean()

# Rolling rank (percentile within window)
df['rolling_pctile'] = df['value'].rolling(90).rank(pct=True)
```

### Multi-Index Operations

```python
# Create multi-index from groupby
multi = df.groupby(['region', 'product']).agg(
    revenue=('amount', 'sum'),
    units=('quantity', 'sum')
)

# Access levels
multi.loc['North']                        # All products in North region
multi.loc[('North', 'Widget')]            # Specific region + product
multi.xs('Widget', level='product')       # All regions for Widget

# Swap and sort levels
multi = multi.swaplevel().sort_index()

# Reset to flat columns
flat = multi.reset_index()

# Stack / unstack (reshape between long and wide)
wide = multi['revenue'].unstack(level='product')    # Products become columns
long = wide.stack()                                  # Back to multi-index
```

### Merge and Join Patterns

```python
# Inner join (only matching rows)
merged = orders.merge(customers, on='customer_id', how='inner')

# Left join with indicator (see which rows matched)
merged = orders.merge(customers, on='customer_id', how='left', indicator=True)
unmatched = merged[merged['_merge'] == 'left_only']

# Join on multiple keys
merged = df1.merge(df2, on=['date', 'region'], how='left')

# Join with different column names
merged = orders.merge(products, left_on='prod_id', right_on='product_id')

# Anti-join (rows in A that have no match in B)
anti = df_a.merge(df_b, on='key', how='left', indicator=True)
anti = anti[anti['_merge'] == 'left_only'].drop(columns='_merge')

# Self-join (compare rows within same table)
df_prev = df[['customer_id', 'order_date', 'amount']].rename(
    columns={'order_date': 'prev_date', 'amount': 'prev_amount'}
)
df_with_prev = df.merge(df_prev, on='customer_id', how='left')
df_with_prev = df_with_prev[df_with_prev['prev_date'] < df_with_prev['order_date']]
```

### Apply and Transform

```python
# transform() returns same-shaped output -- useful for group-level stats on each row
df['group_mean'] = df.groupby('category')['value'].transform('mean')
df['pct_of_group'] = df['value'] / df.groupby('category')['value'].transform('sum')
df['z_within_group'] = df.groupby('category')['value'].transform(
    lambda x: (x - x.mean()) / x.std()
)

# apply() for multi-column group operations
def top_n(group, n=3):
    return group.nlargest(n, 'value')

top3_per_category = df.groupby('category', group_keys=False).apply(top_n, n=3)

# Vectorized operations (prefer these over apply when possible)
# Slow:
df['result'] = df.apply(lambda row: row['a'] * row['b'] + row['c'], axis=1)
# Fast:
df['result'] = df['a'] * df['b'] + df['c']

# np.where for conditional columns (vectorized if/else)
df['tier'] = np.where(df['revenue'] > 10000, 'high', 'low')

# np.select for multiple conditions
conditions = [
    df['revenue'] > 10000,
    df['revenue'] > 5000,
    df['revenue'] > 0,
]
choices = ['high', 'medium', 'low']
df['tier'] = np.select(conditions, choices, default='none')
```

### Memory Optimization for Large Datasets

```python
# Check current memory usage
print(df.memory_usage(deep=True).sum() / 1024**2, "MB")

# Downcast numeric types
df['int_col'] = pd.to_numeric(df['int_col'], downcast='integer')    # int64 -> int8/16/32
df['float_col'] = pd.to_numeric(df['float_col'], downcast='float')  # float64 -> float32

# Use category type for low-cardinality strings
for col in df.select_dtypes(include='object'):
    if df[col].nunique() / len(df) < 0.5:  # Less than 50% unique values
        df[col] = df[col].astype('category')

# Read in chunks for files that exceed memory
chunks = pd.read_csv('huge_file.csv', chunksize=100_000)
results = []
for chunk in chunks:
    processed = chunk.groupby('category')['value'].sum()
    results.append(processed)
final = pd.concat(results).groupby(level=0).sum()

# Specify dtypes at load time (avoids loading as float64/object first)
dtypes = {
    'id': 'int32',
    'category': 'category',
    'value': 'float32',
    'flag': 'bool'
}
df = pd.read_csv('data.csv', dtype=dtypes)

# Use pyarrow backend for better memory efficiency (pandas 2.0+)
df = pd.read_csv('data.csv', engine='pyarrow', dtype_backend='pyarrow')
```

---

## Dashboard and Reporting Patterns

### Executive Dashboard Template

```python
import matplotlib.pyplot as plt
import matplotlib.gridspec as gridspec
from matplotlib.patches import FancyBboxPatch

def executive_dashboard(kpis, trend_df, comparison_df, output='dashboard.png'):
    """
    kpis: dict with keys like {'Revenue': '$1.2M', 'Growth': '+15%', ...}
    trend_df: DataFrame with 'date' and 'value' columns
    comparison_df: DataFrame with 'category' and 'current'/'previous' columns
    """
    fig = plt.figure(figsize=(16, 10))
    gs = gridspec.GridSpec(3, len(kpis), hspace=0.4, wspace=0.3)

    # Row 1: KPI cards
    for i, (label, value) in enumerate(kpis.items()):
        ax = fig.add_subplot(gs[0, i])
        ax.text(0.5, 0.6, value, ha='center', va='center',
                fontsize=28, fontweight='bold', color='#2c3e50')
        ax.text(0.5, 0.2, label, ha='center', va='center',
                fontsize=12, color='#7f8c8d')
        ax.set_xlim(0, 1)
        ax.set_ylim(0, 1)
        ax.axis('off')
        # Card background
        rect = FancyBboxPatch((0.05, 0.05), 0.9, 0.9, boxstyle="round,pad=0.05",
                              facecolor='#f8f9fa', edgecolor='#dee2e6')
        ax.add_patch(rect)

    # Row 2: Trend line
    ax_trend = fig.add_subplot(gs[1, :])
    ax_trend.plot(trend_df['date'], trend_df['value'], linewidth=2, color='steelblue')
    ax_trend.fill_between(trend_df['date'], trend_df['value'], alpha=0.1, color='steelblue')
    ax_trend.set_title('Trend Over Time', fontsize=13, fontweight='bold')
    ax_trend.set_ylabel('Value')

    # Row 3: Period comparison (grouped bar)
    ax_comp = fig.add_subplot(gs[2, :])
    x = range(len(comparison_df))
    width = 0.35
    ax_comp.bar([i - width/2 for i in x], comparison_df['previous'], width,
                label='Previous', color='#bdc3c7')
    ax_comp.bar([i + width/2 for i in x], comparison_df['current'], width,
                label='Current', color='steelblue')
    ax_comp.set_xticks(list(x))
    ax_comp.set_xticklabels(comparison_df['category'], rotation=45, ha='right')
    ax_comp.set_title('Current vs. Previous Period', fontsize=13, fontweight='bold')
    ax_comp.legend()

    plt.savefig(output, dpi=150, bbox_inches='tight', facecolor='white')
    plt.close()
```

### Weekly Metrics Report Template

```python
def weekly_report(df, date_col='date', metric_col='value', group_col=None):
    """Generate a standard weekly metrics summary."""
    df[date_col] = pd.to_datetime(df[date_col])
    df['week'] = df[date_col].dt.isocalendar().week.astype(int)
    df['year'] = df[date_col].dt.year

    current_week = df['week'].max()
    prev_week = current_week - 1

    curr = df[df['week'] == current_week]
    prev = df[df['week'] == prev_week]

    report = {
        'period': f"Week {current_week}",
        'total': curr[metric_col].sum(),
        'mean': curr[metric_col].mean(),
        'median': curr[metric_col].median(),
        'wow_change': (curr[metric_col].sum() - prev[metric_col].sum())
                      / prev[metric_col].sum() * 100
                      if prev[metric_col].sum() != 0 else None,
    }

    if group_col:
        report['by_group'] = curr.groupby(group_col)[metric_col].agg(['sum', 'mean', 'count'])

    # Sparkline trend (last 8 weeks)
    weekly_totals = (
        df.groupby('week')[metric_col].sum()
        .tail(8)
        .reset_index()
    )

    fig, ax = plt.subplots(figsize=(6, 2))
    ax.plot(weekly_totals['week'], weekly_totals[metric_col], marker='o',
            linewidth=2, color='steelblue', markersize=4)
    ax.fill_between(weekly_totals['week'], weekly_totals[metric_col],
                    alpha=0.1, color='steelblue')
    ax.set_title(f'{metric_col.title()} — Last 8 Weeks', fontsize=10)
    ax.tick_params(labelsize=8)
    plt.tight_layout()
    plt.savefig('weekly_sparkline.png', dpi=150, bbox_inches='tight')
    plt.close()

    return report
```

### Anomaly Detection Patterns

```python
def detect_anomalies(series, method='zscore', threshold=3.0, window=30):
    """
    Detect anomalies in a numeric series.

    Methods:
    - 'zscore':  Flag values beyond `threshold` standard deviations from mean
    - 'iqr':     Flag values beyond 1.5x IQR from quartiles
    - 'rolling': Flag values beyond `threshold` std devs from rolling mean
    """
    anomalies = pd.Series(False, index=series.index)

    if method == 'zscore':
        z = (series - series.mean()) / series.std()
        anomalies = z.abs() > threshold

    elif method == 'iqr':
        q1 = series.quantile(0.25)
        q3 = series.quantile(0.75)
        iqr = q3 - q1
        anomalies = (series < q1 - 1.5 * iqr) | (series > q3 + 1.5 * iqr)

    elif method == 'rolling':
        rolling_mean = series.rolling(window, min_periods=5).mean()
        rolling_std = series.rolling(window, min_periods=5).std()
        anomalies = (series - rolling_mean).abs() > threshold * rolling_std

    return anomalies


# Usage: detect and visualize
anomalies = detect_anomalies(df['metric'], method='rolling', threshold=2.5, window=30)

fig, ax = plt.subplots(figsize=(14, 5))
ax.plot(df.index, df['metric'], linewidth=1, color='steelblue', label='Metric')
ax.scatter(df.index[anomalies], df['metric'][anomalies],
           color='red', s=40, zorder=5, label='Anomaly')
ax.legend()
ax.set_title('Anomaly Detection (Rolling Z-Score)', fontsize=14, fontweight='bold')
plt.tight_layout()
plt.savefig('anomalies.png', dpi=150, bbox_inches='tight')
plt.close()

print(f"Detected {anomalies.sum()} anomalies out of {len(series):,} data points")
```

**Method selection guide:**

| Method | Best For | Assumptions | Sensitivity |
|--------|----------|-------------|-------------|
| Z-score | Stationary data with normal distribution | Constant mean and variance | Low (misses local anomalies) |
| IQR | Skewed distributions, outlier screening | None (non-parametric) | Medium |
| Rolling z-score | Time series with trends or seasonality | Local stationarity within window | High (adapts to drift) |

---

## Common Analytics Pitfalls

### Simpson's Paradox

A trend that appears in grouped data reverses when the groups are combined.

```
Department A: Drug works better     (80% vs 70%)
Department B: Drug works better     (50% vs 40%)
Combined:     Drug appears WORSE    (55% vs 60%)  <-- paradox
```

**Why it happens**: Unequal group sizes create a confounding effect. Department B (with lower overall rates) sent most patients to the drug group.

**Prevention**: Always segment data by relevant confounders before drawing conclusions. If aggregate and segmented results disagree, trust the segmented analysis and report the confounding variable.

### Survivorship Bias

Analyzing only entities that "survived" a selection process, ignoring those that dropped out.

**Classic examples:**
- Studying only successful companies to find success patterns (ignoring failed companies with the same patterns)
- Analyzing only current customers to understand satisfaction (ignoring those who already left)
- Looking at fund performance by examining only funds that still exist (dead funds were closed)

**Prevention**: Always ask "what is missing from this dataset?" before drawing conclusions. If possible, include data from non-survivors. Explicitly note the selection criteria and what it excludes.

### Correlation vs. Causation

A statistically significant correlation between X and Y does not mean X causes Y. Possible explanations:

| Explanation | Example |
|-------------|---------|
| X causes Y | Exercise reduces blood pressure |
| Y causes X | Depression reduces exercise (not exercise causes depression) |
| Z causes both | Income drives both education spending AND health outcomes |
| Coincidence | Ice cream sales correlate with drowning deaths (both driven by summer) |

**Prevention**: Establish causation only with randomized controlled experiments (A/B tests). For observational data, state findings as "associated with" not "causes." Look for confounders and test whether the relationship holds when controlling for them.

### Cherry-Picking Time Windows

Selecting a start/end date that makes a metric look better or worse than the true trend.

```python
# Example: same data, different conclusions
# "Revenue up 40%!"    -- comparing Jan (seasonal low) to Dec (seasonal high)
# "Revenue flat."       -- comparing Dec 2024 to Dec 2025 (year-over-year)

# Prevention: always use year-over-year comparison for seasonal data
df['yoy_change'] = df.groupby(df['date'].dt.month)['revenue'].pct_change(periods=12)
```

**Prevention checklist:**
- Compare like-for-like periods (YoY for seasonal businesses)
- Show the full time range, not a selected subset
- Use multiple time windows (WoW, MoM, QoQ, YoY) and note if they disagree
- Include a moving average to show the underlying trend separate from noise

### Small Sample Size Issues

Small samples produce unstable statistics that can flip with just a few more observations.

```python
# Illustrate instability: conversion rates with small vs. large samples
from scipy.stats import beta

# Scenario: 3 conversions out of 10 visitors (30%)
a_small, b_small = 3 + 1, 10 - 3 + 1   # Beta posterior
ci_small = beta.interval(0.95, a_small, b_small)
print(f"n=10:   30% conversion, 95% CI: [{ci_small[0]:.1%}, {ci_small[1]:.1%}]")
# Output: 95% CI: [9.9%, 56.8%]  -- extremely wide, almost useless

# Scenario: 300 conversions out of 1000 visitors (30%)
a_large, b_large = 300 + 1, 1000 - 300 + 1
ci_large = beta.interval(0.95, a_large, b_large)
print(f"n=1000: 30% conversion, 95% CI: [{ci_large[0]:.1%}, {ci_large[1]:.1%}]")
# Output: 95% CI: [27.2%, 32.9%]  -- narrow and actionable
```

**Rules of thumb:**
- n < 30: Do not draw firm conclusions. Report as directional only.
- Conversion rates need hundreds (not dozens) of conversions to stabilize.
- Always report confidence intervals alongside point estimates.
- If sample size is fixed and small, use exact tests (Fisher's exact) rather than approximations (chi-squared).
