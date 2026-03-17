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
