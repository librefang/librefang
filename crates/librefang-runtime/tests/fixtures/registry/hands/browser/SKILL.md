---
name: browser-automation
version: "1.0.0"
description: Playwright-based browser automation patterns for autonomous web interaction
author: LibreFang
tags: [browser, automation, playwright, web, scraping]
tools: [browser_navigate, browser_click, browser_type, browser_screenshot, browser_read_page, browser_close]
runtime: prompt_only
---

# Browser Automation Skill

## Playwright CSS Selector Reference

### Basic Selectors
| Selector | Description | Example |
|----------|-------------|---------|
| `#id` | By ID | `#checkout-btn` |
| `.class` | By class | `.add-to-cart` |
| `tag` | By element | `button`, `input` |
| `[attr=val]` | By attribute | `[data-testid="submit"]` |
| `tag.class` | Combined | `button.primary` |
| `parent child` | Descendant | `div.container button` |
| `parent > child` | Direct child | `ul > li` |
| `:nth-child(n)` | Nth element | `li:nth-child(2)` |
| `:first-child` | First element | `ul > li:first-child` |
| `:last-child` | Last element | `ul > li:last-child` |
| `[attr*=val]` | Attribute contains | `[class*="price"]` |
| `[attr^=val]` | Attribute starts with | `[href^="https"]` |
| `[attr$=val]` | Attribute ends with | `[href$=".pdf"]` |
| `:not(sel)` | Negation | `button:not(.disabled)` |
| `sel1, sel2` | Multiple selectors | `#submit, button[type="submit"]` |

### Form Selectors
| Selector | Use Case |
|----------|----------|
| `input[type="email"]` | Email fields |
| `input[type="password"]` | Password fields |
| `input[type="search"]` | Search boxes |
| `input[name="q"]` | Google/search query |
| `textarea` | Multi-line text areas |
| `select[name="country"]` | Dropdown menus |
| `input[type="checkbox"]` | Checkboxes |
| `input[type="radio"]` | Radio buttons |
| `button[type="submit"]` | Submit buttons |
| `input[type="file"]` | File upload fields |
| `input[type="date"]` | Date pickers |
| `input[type="tel"]` | Phone number fields |
| `input[autocomplete="cc-number"]` | Credit card fields |
| `[contenteditable="true"]` | Rich text editors |

### Navigation Selectors
| Selector | Use Case |
|----------|----------|
| `a[href*="cart"]` | Cart links |
| `a[href*="checkout"]` | Checkout links |
| `a[href*="login"]` | Login links |
| `nav a` | Navigation menu links |
| `.breadcrumb a` | Breadcrumb links |
| `[role="navigation"] a` | ARIA nav links |
| `a[href*="account"]` | Account/profile links |
| `a[href*="register"], a[href*="signup"]` | Registration links |
| `header a[href="/"]` | Logo/home link |
| `footer a` | Footer links |

### E-commerce Selectors
| Selector | Use Case |
|----------|----------|
| `.product-price`, `[data-price]` | Product prices |
| `.add-to-cart`, `#add-to-cart` | Add to cart buttons |
| `.cart-total`, `.order-total` | Cart total |
| `.quantity`, `input[name="quantity"]` | Quantity selectors |
| `.checkout-btn`, `#checkout` | Checkout buttons |
| `[data-product-id]` | Product identifiers |
| `.product-title`, `h1.product-name` | Product names |
| `.product-image img`, `[data-zoom-image]` | Product images |
| `.star-rating`, `[data-rating]` | Review ratings |
| `.in-stock`, `.availability` | Stock status |
| `select[name="size"], .size-selector` | Size selectors |
| `[data-variant], .color-swatch` | Variant selectors |

---

## Generic Selector Strategies (Priority Order)

Use selectors that are resilient to UI redesigns. Prefer semantic and accessibility-based selectors over class names.

### Tier 1 — Test Attributes (most stable)
| Selector | Description |
|----------|-------------|
| `[data-testid="value"]` | Explicit test ID — survives refactors |
| `[data-test="value"]` | Alternative test attribute convention |
| `[data-cy="value"]` | Cypress test attribute |
| `[data-qa="value"]` | QA-specific test attribute |

### Tier 2 — Accessibility Attributes
| Selector | Description |
|----------|-------------|
| `[aria-label="Search"]` | Accessible name, framework-agnostic |
| `[aria-labelledby="id"]` | References a labelling element |
| `[role="button"]` | ARIA role — semantic intent |
| `[role="link"]` | ARIA link role |
| `[role="textbox"]` | ARIA textbox role |
| `[role="dialog"]` | Modals and popups |
| `[role="navigation"]` | Navigation landmarks |
| `[role="search"]` | Search landmarks |
| `[aria-expanded="true"]` | Open dropdowns/menus |
| `[aria-selected="true"]` | Selected tabs/options |
| `[aria-checked="true"]` | Checked checkboxes/radios |
| `[aria-disabled="true"]` | Disabled elements (do not click) |

### Tier 3 — Semantic HTML
| Selector | Description |
|----------|-------------|
| `button[type="submit"]` | Form submit buttons |
| `input[name="fieldname"]` | Form fields by name |
| `input[type="email"]` | Email input by type |
| `label[for="fieldid"]` | Label linked to input |
| `nav a` | Navigation links |
| `main`, `article`, `section` | Content landmarks |
| `header`, `footer` | Page structure |
| `h1`, `h2`, `h3` | Headings for orientation |

### Tier 4 — ID and Visible Text
| Strategy | When to use |
|----------|-------------|
| `#unique-id` | When ID is human-readable and stable |
| Visible text content | When no good attribute selectors exist |
| `a:has-text("Sign In")` | Playwright-specific text matching |

### Tier 5 — Class Selectors (least stable)
| Risk | Pattern |
|------|---------|
| Low risk | `.btn-primary`, `.nav-link` (design-system classes) |
| Medium risk | `.header-search-input` (component-specific) |
| High risk | `.css-1a2b3c`, `.sc-fAbCdE` (auto-generated by CSS-in-JS) |

**Rule:** Never rely on auto-generated class names (random strings like `.css-xyz123`). These change on every build.

## Accessibility-Based Interaction Patterns

Modern web apps expose accessibility attributes that are more stable than CSS classes.

### Finding Interactive Elements by Role
```
Buttons:     [role="button"], button
Links:       [role="link"], a[href]
Text inputs: [role="textbox"], input[type="text"], textarea
Checkboxes:  [role="checkbox"], input[type="checkbox"]
Radio:       [role="radio"], input[type="radio"]
Comboboxes:  [role="combobox"] (autocomplete/typeahead fields)
Tabs:        [role="tab"] (tab navigation)
Menus:       [role="menu"], [role="menuitem"]
Dialogs:     [role="dialog"], [role="alertdialog"]
```

### Reading Page Structure via Landmarks
```
[role="banner"]       → site header (logo, global nav)
[role="navigation"]   → navigation sections
[role="main"]         → primary page content
[role="search"]       → search functionality
[role="contentinfo"]  → footer (copyright, legal links)
[role="complementary"] → sidebar content
[role="form"]         → form regions
```

### Label-Based Field Identification
```
Instead of guessing input selectors, find labels first:
1. browser_read_page → look for label text (e.g., "Email Address")
2. Use: label:has-text("Email") + input  (sibling)
   Or:  input[aria-label="Email Address"]
   Or:  #<id-from-label-for-attribute>
```

## SPA Framework Detection & Handling

### Detecting the Framework
| Signal | Framework | Notes |
|--------|-----------|-------|
| `<div id="root">` or `<div id="__next">` | React / Next.js | Content rendered client-side |
| `<div id="app">` with `data-v-` attributes | Vue.js / Nuxt | `data-v-xxxxx` are scoped style markers |
| `<app-root>` or custom element tags | Angular | Uses web component-like tags |
| `<div id="svelte">` or compiled class names | Svelte / SvelteKit | Minimal runtime footprint |
| URL contains `#/` hash routing | Any SPA | Client-side routing via hash |
| `__NEXT_DATA__` script tag | Next.js | Server-side rendering with hydration |
| `__NUXT__` or `__NUXT_DATA__` in page | Nuxt.js | Vue SSR framework |

### Framework-Specific Interaction Tips

**React apps:**
- State updates are batched — wait 500ms-2s after interactions for re-renders
- Look for `data-testid` attributes (common in React Testing Library projects)
- Portal-rendered content (modals, tooltips) may be at the end of `<body>`, not nested in the component tree
- React-Select dropdowns: click the container, then look for `[class*="option"]` in the menu that appears

**Vue apps:**
- `v-if` elements may not exist in DOM until conditions are met — re-read page after state changes
- Vue transitions: wait for CSS transitions to complete before interacting
- Vuetify/Element UI components have predictable class prefixes (`.v-btn`, `.el-input`)

**Angular apps:**
- Elements often have `_ngcontent-` or `_nghost-` attributes (do not use these as selectors — they change per build)
- Angular Material components: use `[role]` and `[aria-label]` attributes instead of classes
- Forms may use reactive validation — errors appear only after interaction (`blur` event)

**General SPA rules:**
- After clicking a navigation element, wait 1-3 seconds before reading the page
- If content is missing, check for loading indicators: `.loading`, `.spinner`, `[aria-busy="true"]`, `.skeleton`
- Retry `browser_read_page` up to 3 times with 2-second intervals before giving up
- URL changes without full page reload confirm SPA routing — do not expect `browser_navigate` events

## Site-Specific Selector Patterns

These are reference selectors for common sites. They change frequently — always verify with `browser_read_page` if a selector fails, then construct a fresh selector from the live DOM.

### Google Search
| Element | Selector |
|---------|----------|
| Search input | `input[name="q"]`, `textarea[name="q"]` |
| Search button | `input[name="btnK"]`, `button[type="submit"]` |
| Result titles | `h3` (within `#search`) |
| Result links | `#search a[href^="http"]` |
| "Next" pagination | `a#pnnext` |

### Amazon
| Element | Selector |
|---------|----------|
| Search input | `#twotabsearchtextbox` |
| Search button | `#nav-search-submit-button` |
| Add to cart | `#add-to-cart-button` |
| Quantity dropdown | `#quantity` |
| Cart count | `#nav-cart-count` |

### GitHub
| Element | Selector |
|---------|----------|
| Search | `input[name="q"]` |
| Repository name | `[itemprop="name"] a` |
| Star button | `button[aria-label*="Star"]` |
| Submit button | `button[type="submit"]` |

Note: When a saved selector fails, use `browser_read_page` to discover the current DOM, then build a new selector from live content. Prefer `[data-testid]`, `[aria-label]`, or visible text over fragile class-based selectors.

---

## Common Workflows

### Product Search & Purchase
```
1. browser_navigate → store homepage
2. browser_type → search box with product name
3. browser_click → search button or press Enter
4. browser_read_page → scan results
5. browser_click → desired product
6. browser_read_page → verify product details & price
7. browser_click → "Add to Cart"
8. browser_navigate → cart page
9. browser_read_page → verify cart contents & total
10. STOP → Report to user, wait for approval
11. browser_click → "Proceed to Checkout" (only after approval)
```

### Account Login
```
1. browser_navigate → login page
2. browser_read_page → identify form fields and any CAPTCHA
3. browser_type → email/username field
4. browser_type → password field
5. browser_click → login/submit button
6. browser_read_page → verify successful login (check for dashboard/profile elements)
7. If MFA required → inform user, wait for code input
```

### Form Submission
```
1. browser_navigate → form page
2. browser_read_page → understand form structure
3. browser_type → fill each field sequentially
4. browser_click → checkboxes/radio buttons as needed
5. browser_screenshot → visual verification before submit
6. browser_click → submit button
7. browser_read_page → verify confirmation
```

### Price Comparison
```
1. For each store:
   a. browser_navigate → store URL
   b. browser_type → search query
   c. browser_read_page → extract prices
   d. memory_store → save price data
2. memory_recall → compare all prices
3. Report findings to user
```

### Multi-Page Data Extraction
```
1. browser_navigate → starting page
2. browser_read_page → extract data from current page
3. memory_store → save extracted data
4. Check for pagination:
   a. browser_click → "Next" button or page number link
   b. browser_read_page → verify new page loaded (check for changed content)
   c. Repeat from step 2
5. If no more pages → compile and report results
```

### Account Registration
```
1. browser_navigate → registration page
2. browser_read_page → identify required fields
3. browser_type → fill name, email, password fields sequentially
4. browser_click → accept terms checkbox (if required)
5. browser_screenshot → verify all fields before submission
6. browser_click → submit/register button
7. browser_read_page → check for:
   - Success page → registration complete
   - Email verification prompt → inform user
   - Validation errors → read errors, correct fields, retry
```

### File Download Monitoring
```
1. browser_navigate → page with download link
2. browser_read_page → identify download button/link
3. browser_click → download trigger
4. browser_read_page → check for download confirmation or redirect
5. If download requires additional steps (accept terms, choose format):
   a. browser_click → required selections
   b. browser_click → final download button
6. Report download status to user
```

---

## Wait Strategies & Timing

### When to Wait
Proper waiting prevents most automation failures. Never use fixed sleep times when a condition-based wait is possible.

| Scenario | Strategy | Notes |
|----------|----------|-------|
| Page navigation | Wait for load event | `browser_navigate` handles this automatically |
| After clicking link | Read page to confirm new content | Check for expected elements on destination |
| AJAX/dynamic content | Re-read page after delay | Some SPAs load content asynchronously |
| Form submission | Read page for confirmation | Check for success message or redirect |
| Slow networks | Retry with backoff | 3s, 6s, 12s intervals |
| Animation/transition | Brief pause before interaction | Modal fade-in, dropdown expansion |

### Detecting Page Load Completion
```
After browser_navigate or browser_click that triggers navigation:
1. browser_read_page → check if expected content is present
2. If content missing → wait 2-3 seconds → browser_read_page again
3. If still missing after 3 retries → page may have changed structure
4. Use browser_screenshot to visually confirm page state
```

### SPA (Single Page Application) Handling
SPAs (React, Angular, Vue, Svelte) do not trigger traditional page loads. Client-side routing means the browser URL changes but no network navigation occurs.

```
1. browser_click → triggers route change (URL updates but no page reload)
2. browser_read_page → may return stale content from previous view
3. Check for loading indicators in the output:
   - Text: "Loading...", "Please wait", skeleton placeholders
   - Attributes: [aria-busy="true"]
   - Classes: .loading, .spinner, .skeleton, .placeholder
4. If loading detected OR content stale → wait 2 seconds
5. browser_read_page → retry (attempt 2 of 3)
6. If still stale → wait 3 seconds → browser_read_page (attempt 3 of 3)
7. If content never updates:
   a. browser_screenshot → check if content is visually present but not captured as text
   b. The content may be inside an iframe or shadow DOM — try alternative access
   c. Report the issue to the user with the screenshot
```

### Iframe Content Access
```
When target content is inside an iframe:
1. browser_read_page → look for <iframe> elements and their src attributes
2. browser_navigate → directly to the iframe src URL (if same-origin)
3. Interact with the content normally
4. browser_navigate → back to the parent page when done
Note: Cross-origin iframes may block direct access. Inform the user if this occurs.
```

### Shadow DOM Awareness
```
Web components using shadow DOM hide their internals from normal CSS selectors:
1. If a known element is not found by any selector, suspect shadow DOM
2. browser_screenshot → visually confirm the element exists on the page
3. Try interacting via visible text content (may pierce shadow boundaries)
4. If interaction fails, inform the user that the element is inside a shadow root
```

---

## Error Recovery Strategies

### Error Recovery Decision Tree
When any interaction fails, walk through this decision tree top-to-bottom:

```
INTERACTION FAILED
│
├─ Is this the correct page?
│  ├─ NO → browser_read_page to check URL
│  │       ├─ Redirected to login? → re-authenticate, then retry
│  │       ├─ Redirected to error page? → handle HTTP error (see below)
│  │       └─ Wrong page entirely? → browser_navigate to correct URL
│  └─ YES ↓
│
├─ Is an overlay blocking the element?
│  ├─ YES → dismiss overlay (cookie banner, modal, chat widget)
│  │        then retry the original interaction
│  └─ NO ↓
│
├─ Does the element exist in the DOM?
│  ├─ NO → page may not have finished rendering
│  │       ├─ Wait 2 seconds → browser_read_page → retry (up to 3 times)
│  │       ├─ Scroll the page to trigger lazy loading → retry
│  │       ├─ Try alternative selectors (see priority order below)
│  │       └─ Still not found? → browser_screenshot → report to user
│  └─ YES ↓
│
├─ Is the element visible and interactive?
│  ├─ Disabled ([disabled], [aria-disabled="true"]) → inform user, cannot interact
│  ├─ Hidden (display:none, off-screen) → may be inside collapsed section, try expanding
│  ├─ Covered by another element → identify and dismiss the covering element
│  └─ YES ↓
│
├─ Did the click/type register?
│  ├─ NO → JavaScript may not have attached handlers yet
│  │       ├─ Wait 2 seconds → retry
│  │       ├─ Try clicking a more specific child element
│  │       └─ Try clicking by visible text instead of CSS selector
│  └─ YES ↓
│
└─ Did the expected state change occur?
   ├─ NO → SPA may need time to re-render
   │       ├─ Wait 2-3 seconds → browser_read_page to verify
   │       ├─ Check for loading indicators ([aria-busy], .spinner)
   │       └─ After 3 retries, browser_screenshot → report to user
   └─ YES → continue to next step
```

### Selector Fallback Order
When the primary selector fails, try alternatives in this order:
```
1. [data-testid="..."], [data-test="..."], [data-cy="..."]  — test attributes
2. [aria-label="..."], [role="button"]                       — accessibility
3. Visible text content: a:has-text("Sign In")               — human-readable
4. input[name="..."], input[type="..."]                      — form semantics
5. #id                                                        — unique ID
6. [class*="keyword"]                                         — partial class match (last resort)
```

### Quick Reference
| Error | Recovery |
|-------|----------|
| Element not found | Walk selector fallback order, scroll page, screenshot |
| Page timeout | Retry URL once, try base domain, report to user |
| Login required | Inform user, ask for credentials |
| CAPTCHA | Screenshot and inform user — cannot solve |
| Pop-up/modal | Dismiss first, then retry original action |
| Cookie consent | Click "Accept All" or dismiss banner |
| Rate limited (429) | Wait 30s, retry; after 3 failures, stop and report |
| Session expired | Detect login redirect, re-authenticate, resume |
| Wrong page | Verify URL, navigate back or to correct page |
| Empty SPA content | Wait 3-5s for render, retry read up to 3 times |

### Navigation Failure Recovery
```
1. Timeout on browser_navigate:
   a. Retry the same URL once
   b. If still fails, check if URL is valid (no typos, correct protocol)
   c. Try simplified URL (remove query params, try base domain)
   d. Report connectivity issue to user

2. Unexpected redirect:
   a. browser_read_page → check current URL and content
   b. If redirected to login → handle login flow
   c. If redirected to error page → report the error code and message
   d. If redirected to different page → assess if it is relevant, otherwise navigate back

3. HTTP errors observed in page content:
   - 403 Forbidden → site may be blocking automation, inform user
   - 404 Not Found → URL is stale or incorrect, try searching for the correct page
   - 429 Too Many Requests → wait 60 seconds, retry with longer intervals
   - 500/502/503 → server issue, retry after 30 seconds (max 3 retries)
```

### Stale Element Recovery (SPA-Specific)
Elements become stale when the page re-renders — common in React, Vue, and Angular:
```
1. Identify the stale interaction (click that produced no result or error)
2. browser_read_page → get fresh DOM snapshot
3. Check if the element's selector still matches in the new DOM
4. If not, construct a new selector from the fresh page content
5. Retry the interaction with the updated selector
6. If element has moved, use browser_screenshot to find its new location
```

### Pop-up and Overlay Dismissal
```
Order of priority when overlays block interaction:
1. Cookie consent banners:
   - Selectors: [aria-label*="cookie" i] button, #onetrust-accept-btn-handler
   - Text: "Accept All", "Accept Cookies", "Agree", "OK", "Got it"
   - Fallback: .cookie-banner .close, #cookie-close, [aria-label="Close"]

2. Newsletter/subscription modals:
   - Selectors: [role="dialog"] [aria-label="Close"], .modal .close, button.dismiss
   - Fallback: press Escape key (via browser_type on body element)

3. Chat widgets:
   - Usually do not block (z-index layer); if blocking, click minimize/close

4. Age verification gates:
   - Click "Yes" / "I am over 18" / "Enter"
   - May need to select a birth date from dropdowns

5. App install banners:
   - Look for "Continue in browser", "Not now", or close/dismiss buttons

6. Browser-level permission prompts:
   - Auto-dismissed by Playwright context settings
```

---

## Anti-Detection Techniques

### Behavioral Patterns
Automation detection looks for non-human interaction patterns. Mitigate by:
```
1. TIMING: Do not interact instantly after page load
   - Wait 1-3 seconds before first interaction
   - Insert 0.5-2 second gaps between form field entries
   - Vary timing (not perfectly uniform intervals)

2. NAVIGATION: Follow natural browsing patterns
   - Visit homepage before deep URLs when possible
   - Click through navigation instead of using direct URLs
   - Scroll before interacting with below-the-fold content

3. INPUT: Simulate realistic behavior
   - Type character by character (browser_type handles this)
   - Click buttons rather than submitting forms programmatically
   - Do not fill hidden honeypot fields (see below)
   - Do not request pages faster than 1 per 3 seconds on the same domain
```

### Honeypot Field Detection
```
Do NOT fill fields that have:
  - style="display: none" or style="visibility: hidden"
  - class="hidden", class="d-none", class="sr-only"
  - type="hidden" (unless it is a legitimate CSRF token or form ID)
  - Position: absolute with left: -9999px (off-screen placement)

Use browser_read_page to inspect field visibility before filling.
```

---

## Screenshot & Content Extraction

### When to Take Screenshots
| Situation | Purpose |
|-----------|---------|
| Before form submission | Visual verification of filled data |
| After login attempt | Confirm success or capture error state |
| When element not found | See actual page state for debugging |
| Price/product comparison | Visual record for user |
| CAPTCHA encountered | Show user what needs solving |
| Before financial transaction | Proof of cart/payment details |
| Unexpected page state | Diagnose navigation or rendering issues |

### Content Extraction Patterns
```
Tables:  browser_read_page → identify table boundaries → parse rows/columns → memory_store
Data:    browser_read_page → search for labels ("Price:", "In Stock", "Rating:") → extract adjacent values
Dynamic: browser_read_page → if "Loading..." or skeleton → wait 2-3s → retry
Scroll:  extract visible data → scroll down → browser_read_page → repeat until complete (max 10 cycles)
```

---

## Form Filling & Interaction Sequences

### Dropdown / Select Menus
```
Standard HTML <select>:
  browser_click → the <select> element to open it
  browser_click → the <option> with desired value

Custom dropdown (div-based):
  1. browser_click → the dropdown trigger element (.dropdown-toggle, .select-wrapper)
  2. browser_read_page → find the dropdown options that appeared
  3. browser_click → the desired option from the expanded list
```

### Date Pickers
```
Native HTML date input (input[type="date"]):
  browser_type → the date value in YYYY-MM-DD format directly into the input

Custom calendar widget:
  1. browser_click → date input to open calendar
  2. browser_click → month/year navigation arrows to reach target month
  3. browser_click → the target day cell
  4. browser_read_page → verify selected date
```

### File Uploads
```
Standard file input (input[type="file"]):
  - Playwright can set file input values directly
  - Not always available via browser_type — inform user if upload needed

Drag-and-drop upload zones:
  - Usually have a fallback "Browse files" button/link
  - browser_click → "Browse" or "Choose file" fallback button
```

### Multi-Step Forms (Wizards)
```
1. browser_read_page → identify current step and total steps
2. Fill current step fields with browser_type and browser_click
3. browser_click → "Next" / "Continue" button
4. browser_read_page → verify progression to next step
5. If validation error appears:
   a. browser_read_page → identify error messages
   b. Fix the flagged fields
   c. browser_click → "Next" again
6. Repeat until final step
7. browser_screenshot → capture summary/review page
8. STOP → get user approval before final submission
```

### Autocomplete / Typeahead Fields
```
1. browser_type → partial text into the input field
2. Wait 1-2 seconds for suggestion dropdown to appear
3. browser_read_page → check if suggestions loaded
4. browser_click → the desired suggestion from the dropdown
5. browser_read_page → verify the field populated correctly
```

### Checkbox and Radio Button Groups
```
Checkboxes (multi-select):
  browser_click → each desired checkbox
  Use browser_read_page to check current state ([checked] attribute)
  Do NOT click already-checked boxes unless intending to uncheck

Radio buttons (single-select):
  browser_click → the desired option
  Only one can be active — clicking a new one deselects the previous
```

---

## Multi-Tab and Popup Handling

### Links That Open New Tabs
```
When browser_click opens a new tab (target="_blank" links):
1. The new tab becomes the active context
2. browser_read_page → verify content in new tab
3. Complete actions in the new tab
4. browser_close → close the new tab to return to the original
5. browser_read_page → verify original tab is still in expected state
```

### Authentication Popups (OAuth, SSO)
```
OAuth/Social login flows often open a popup:
1. browser_click → "Sign in with Google/GitHub/etc."
2. New popup/tab opens with the provider's login page
3. browser_type → credentials in the popup
4. browser_click → authorize/allow button
5. Popup closes automatically, original page refreshes
6. browser_read_page → verify login success on original page
```

### Multiple Window Coordination
```
When workflow requires information from multiple pages:
1. Open first page → extract needed data → memory_store
2. browser_navigate → second page (replaces current)
3. Extract data from second page → memory_store
4. Use memory_recall to combine data from both sources
5. browser_navigate → return to original page if needed

Avoid keeping multiple tabs open simultaneously when possible —
single-tab workflows are more reliable and predictable.
```

---

## Cookie and Session Management

### Session Persistence
```
Playwright browser sessions persist cookies within a single session:
- Logging in once is enough for subsequent navigations to the same site
- Cookies expire when the browser context is closed (browser_close)
- For long workflows, periodically verify session is still active:
  1. browser_navigate → a page that requires authentication
  2. browser_read_page → check for login page redirect
  3. If redirected to login → session expired, re-authenticate
```

### Cookie Consent Handling
```
Most sites display cookie banners on first visit. Handle immediately:
1. browser_read_page → check for cookie banner presence
2. Look for accept buttons with these common selectors:
   - #cookie-accept, #accept-cookies, #onetrust-accept-btn-handler
   - button[data-action="accept"], .cookie-consent .accept
   - Text content: "Accept All", "Accept Cookies", "I Agree", "OK"
3. browser_click → the accept button
4. browser_read_page → verify banner dismissed
5. If banner persists → try clicking the close/X button instead
```

### Geo-Restricted Content
```
When content varies by location:
1. browser_read_page → check for location selection prompts
2. If location selector present:
   a. browser_click → country/region dropdown
   b. browser_click → desired country/region
   c. browser_read_page → verify content updated for selected region
3. Some sites redirect to country-specific domains (.co.uk, .de, etc.)
   - Navigate directly to the correct country domain
```

---

## Performance Optimization

### Minimizing Page Reads
```
browser_read_page is the most expensive operation (returns full page text).
Optimize by:
1. Use browser_read_page once after navigation, extract ALL needed data
2. Use browser_screenshot for quick visual checks instead of full reads
3. After clicking a button, only read the page if you need to verify
   the result or extract new data
4. Cache page content in memory_store if you will reference it later
```

### Efficient Navigation
```
1. Use direct URLs when possible instead of click-through navigation:
   BAD:  homepage → click "Products" → click "Electronics" → click "Phones"
   GOOD: browser_navigate → /products/electronics/phones

2. Use URL patterns for pagination:
   If page 1 is /results?page=1, go directly to /results?page=5
   instead of clicking "Next" five times

3. Combine operations:
   Instead of: navigate → read → navigate → read → navigate → read
   Use: navigate → read + extract all → navigate next → read + extract all
```

### Handling Lazy-Loaded Content
```
Many modern sites load content as the user scrolls (infinite scroll):
1. browser_read_page → get initially loaded content
2. If you need more items:
   a. browser_click → a far-down element to trigger scroll loading
   b. Wait 2-3 seconds for new content to render
   c. browser_read_page → extract newly loaded content
   d. Repeat until sufficient data collected
3. Set a maximum iteration count (e.g., 10 scroll cycles)
   to prevent infinite loops on endlessly scrolling pages
```

### Rate Limiting Awareness
```
Respect site rate limits to avoid IP blocks:
- Space requests to same domain by at least 2-3 seconds
- If you receive a 429 error or see "Too many requests":
  1. Wait 30-60 seconds before next request
  2. Double the wait time on each subsequent 429
  3. After 3 consecutive 429s, inform user and stop
- Watch for soft rate limiting signals:
  - CAPTCHA appearing mid-session
  - Results becoming empty or different
  - Redirect to a "verify you are human" page
```

---

## Common Failure Modes & Debugging

### Diagnosis Checklist
When an interaction fails, check in this order:
```
1. CORRECT PAGE?
   browser_read_page → confirm you are on the expected URL/page

2. ELEMENT EXISTS?
   browser_read_page → search output for the target element's text or nearby text

3. ELEMENT VISIBLE?
   browser_screenshot → check if element is visible (not behind overlay, not off-screen)

4. ELEMENT BLOCKED?
   Look for cookie banners, modals, chat widgets covering the element

5. ELEMENT INTERACTIVE?
   Check if element is disabled, greyed out, or read-only

6. PAGE FULLY LOADED?
   Look for loading indicators in page content (spinner text, "Loading...")

7. SESSION VALID?
   Check if you have been redirected to a login page
```

### Common Failure Patterns

**Clicking does nothing:**
```
Causes:
  - Element is covered by an overlay (cookie banner, modal)
  - Element is outside the viewport (needs scroll)
  - Element is disabled or has pointer-events: none
  - JavaScript click handler has not attached yet (page still loading)
  - Clicking on a wrapper div instead of the actual button

Fixes:
  1. Dismiss any overlays first
  2. Scroll to the element before clicking
  3. Wait 2 seconds and retry
  4. Try a more specific selector targeting the inner clickable element
```

**Form submission fails silently:**
```
Causes:
  - Required field left empty (client-side validation blocked submit)
  - Hidden honeypot field was filled (bot detection)
  - CSRF token expired (session timeout)
  - JavaScript form validation prevents submission

Fixes:
  1. browser_read_page → check for inline validation error messages
  2. browser_screenshot → look for highlighted required fields
  3. Verify all required fields are filled (check for asterisks or "required" labels)
  4. Try refreshing the page and filling the form again (new CSRF token)
```

**Page content appears empty or minimal:**
```
Causes:
  - Site requires JavaScript rendering (content injected by React/Vue/Angular)
  - Content is behind an authentication wall
  - Site is geo-blocked or IP-blocked
  - Content loads via AJAX after initial page load

Fixes:
  1. Wait 3-5 seconds after navigation, then browser_read_page
  2. browser_screenshot → check if content is visually present but not in text
  3. Check if login is required
  4. Try a different URL or approach
```

**Redirect loop or unexpected page:**
```
Causes:
  - Session expired → site redirects to login → tries to redirect back → loop
  - Geo-redirect based on IP location
  - A/B testing serves different page version
  - Site requires cookie acceptance before showing content

Fixes:
  1. browser_read_page → check current URL
  2. Handle cookie consent if prompted
  3. Re-authenticate if on login page
  4. Try navigating to the base domain first, then to the target page
```

---

## Security Checklist

### Before Entering Credentials
- Verify the domain matches the expected site (check for typosquatting)
- Confirm the page is served over HTTPS
- Look for suspicious elements (extra fields, unusual layout)
- Never enter credentials on HTTP pages
- Warn user about any certificate warnings or security indicators

### During Sensitive Operations
- Never store passwords in memory_store — use them only in browser_type
- Never auto-approve financial transactions — always STOP and confirm with user
- Report any unexpected redirects during authentication
- Log out of sessions when workflow is complete

### Phishing Indicators to Watch For
| Indicator | Example | Action |
|-----------|---------|--------|
| Misspelled domain | amaz0n.com, gooogle.com | STOP and warn user |
| Unusual TLD | amazon.xyz, paypal.ru | STOP and warn user |
| HTTP on login page | http://bank.com/login | STOP and warn user |
| Extra form fields | SSN on a retail login | STOP and warn user |
| Suspicious redirects | Login redirects to different domain | STOP and warn user |
| Urgency language | "Account suspended, act now" | Inform user, suggest verifying |

### Data Handling Rules
```
SAFE to store in memory_store:
  - Product names, prices, descriptions
  - Public profile information
  - Search results and comparisons
  - Order confirmation numbers
  - Non-sensitive form field values

NEVER store in memory_store:
  - Passwords, PINs, security codes
  - Credit card numbers or CVVs
  - Social Security or government ID numbers
  - Authentication tokens or session cookies
  - Private messages or confidential communications
```
