# Broward County Brain Plan

## Purpose

This document defines a concrete plan for turning Broward County public and institutional data into a county-scale intelligence system built on LibreFang. The near-term goal is to create an internal strategic asset for fundraising, market planning, and institutional expansion work. The longer-term goal is to turn the same system into a repeatable custom intelligence offering for hospitals, nonprofits, funders, and governments.

The system is not a raw data warehouse. It is a source-backed county intelligence graph with AI agents that can answer questions, generate briefs, monitor change, and support human decision-making.

## Core Thesis

The opportunity is not "sell access to public records."

The opportunity is:

- ingest fragmented county data once
- normalize it into a durable graph of people, organizations, properties, institutions, places, and events
- layer agent workflows on top of that graph
- turn raw records into source-backed intelligence products

For the initial Broward use case, the system should support:

- fundraising strategy
- major gift research
- institutional market planning
- Cleveland Clinic South Florida expansion planning
- government and funder intelligence projects

## Strategic Outcomes

The Broward County Brain should enable five categories of output:

1. Property and entity intelligence
2. Prospect and donor research
3. Place-based demographic and neighborhood analysis
4. Institutional and market planning
5. Continuous monitoring and alerting

In practical terms, the system should be able to answer questions like:

- Which geographies in Broward show the highest combination of wealth, development activity, and philanthropic potential?
- Which people, entities, and trusts are linked to the most valuable real-estate holdings in a target area?
- What real-estate, organizational, and demographic signals support expansion in a target corridor?
- What changed this week in deeds, mortgages, liens, entity activity, or development patterns that matters to fundraising or planning?
- Build a source-backed briefing on a prospect, institution, municipality, or neighborhood.

## Product Direction

The Broward County Brain should be treated as an internal operating model first and a product second.

### Internal Uses

- major gift prospect research
- campaign planning
- geographic portfolio prioritization
- expansion and service-area analysis
- executive and board briefing support
- institutional partnership mapping

### External Uses

- custom county intelligence projects for hospitals
- nonprofit prospect intelligence systems
- place-based strategy support for governments and funders
- AI research workspaces tailored to specific institutional missions

The product being monetized is not the data itself. The product is the intelligence layer: source-backed briefs, agent workflows, monitoring, relationship maps, and decision support.

## Data Strategy

The data strategy should use a layered model.

### Layer 1: Source of Record

Structured public datasets that provide deterministic truth.

- Broward County recorder bulk files
- Broward property appraiser or assessor data
- Census and ACS data
- parcel and geography reference data
- state business entity records
- IRS nonprofit records
- municipal and institutional public records

### Layer 2: Research Enrichment

Public or licensed sources that add context.

- official websites
- press releases
- local and regional news
- board rosters
- public leadership bios
- licensed firmographic or philanthropic data where available

### Layer 3: Evidence Layer

Primary documents and originals used when metadata is not enough.

- recorder image archives
- OCR outputs for selected documents
- PDFs, reports, and official plans

### Layer 4: Intelligence Layer

Synthesized, traceable objects for search and agent use.

- prospect briefs
- institutional briefs
- neighborhood summaries
- opportunity memos
- relationship summaries
- risk and confidence notes

## Broward Recorder Data: What It Gives Us

The Broward recorder SFTP feed appears to provide two relevant classes of data:

- yearly text exports for broad historical coverage
- rolling daily feeds that include text plus image archives

The text exports contain structured metadata for recorded instruments and related references. Based on sample files already inspected, the text files provide enough to build a high-value metadata and relationship system without needing to OCR every original document upfront.

### Text Files Observed

- `doc` records: recording number, dates, times, document type, amounts, page counts, case references, and other administrative fields
- `nme` records: party names linked to recording numbers and role/order metadata
- `lgl` records: legal descriptions and parcel identifiers linked to recording numbers
- `lnk` records: linkage between related records

### What Text Alone Supports

- property timelines
- ownership and mortgage histories
- lien and litigation indexing
- party and entity matching
- parcel-centric and record-centric CRM enrichment
- broad semantic retrieval over synthesized record summaries

### What Requires Originals

- full document body text
- clause extraction
- agreement summaries
- signature and exhibit review
- page-level evidence
- exact interpretation of ambiguous instruments

### Practical Recorder Ingestion Policy

The recorder feed should be treated as a two-tier system:

- ingest all structured text as the low-cost base layer
- ingest originals selectively for high-value or high-ambiguity records

This keeps costs reasonable while preserving a path to deep document intelligence.

## Required Data Sources

The first version should focus on a bounded but powerful source set.

### Phase 1 Sources

- Broward recorder text exports
- Broward daily recorder text updates
- Broward property appraiser or assessor parcel and valuation data
- Census and ACS tract/block-group data
- municipal boundary and geography files

### Phase 2 Sources

- Florida business entity records
- IRS exempt organization and Form 990 data
- hospital and health-system location data
- school, municipality, and major employer reference sets
- local news and public institutional websites

### Phase 3 Sources

- selected recorder originals and OCR
- permits and development records where available
- board and executive affiliation datasets
- licensed enrichment and wealth context sources if needed
- internal CRM and fundraising data under strict governance

## Canonical Data Model

The system should use a canonical graph with structured tables under it. Do not rely on embeddings alone.

### Core Entities

- `person`
- `organization`
- `household`
- `property`
- `parcel`
- `document`
- `document_type`
- `case_reference`
- `institution`
- `place`
- `neighborhood`
- `census_area`
- `event`
- `source_record`

### Core Relationship Types

- person to organization
- person to property
- person to household
- organization to property
- organization to organization
- property to parcel
- document to property
- document to person
- document to organization
- property to census area
- property to neighborhood
- institution to geography
- institution to organization
- event to all affected entities

### Suggested Structured Tables

- `documents`
- `document_parties`
- `document_properties`
- `properties`
- `parcels`
- `party_identities`
- `organizations`
- `households`
- `ownership_events`
- `mortgage_events`
- `lien_events`
- `litigation_events`
- `institution_locations`
- `geography_facts`
- `source_citations`
- `entity_aliases`
- `entity_resolutions`

### Key Identifiers

- recorder `recording_number`
- parcel or folio identifier
- normalized legal description hash
- normalized person and organization keys
- Census tract and block-group IDs
- source document IDs and URLs

## Entity Resolution

Entity resolution is one of the highest-value pieces of the system.

Without it, the result is a large database.
With it, the result is intelligence.

Resolution should support:

- person name normalization
- organization name normalization
- trust and LLC matching
- household grouping where lawfully sourced
- cross-document linkage
- confidence scoring
- alias retention rather than destructive merging

Every resolved entity should preserve:

- canonical display name
- alternate names
- source evidence
- confidence score
- last verified date

## Search and Retrieval Architecture

The system should support three retrieval modes.

### 1. Structured Retrieval

Use SQL or graph queries first for:

- exact property lookups
- parcel history
- party history
- document filtering
- date ranges
- document type filters
- geographic aggregations

### 2. Full-Text Retrieval

Use full-text search for:

- names
- legal descriptions
- case numbers
- organization names
- news and public bios

### 3. Semantic Retrieval

Use vector search for:

- synthesized prospect summaries
- neighborhood briefs
- relationship narratives
- OCR chunks for selected originals
- public narrative documents and reports

Embeddings should be created from synthesized, human-readable summaries, not only raw rows. Raw rows remain useful as evidence, but synthesized records dramatically improve agent retrieval quality.

## AI Agent Design

The system should expose task-specific agents with controlled tool access.

### Primary Agents

- `research_agent`
- `prospect_agent`
- `territory_agent`
- `property_agent`
- `institution_agent`
- `expansion_agent`
- `watchlist_agent`
- `briefing_agent`

### Agent Responsibilities

`research_agent`
- compiles source-backed dossiers on people, organizations, or institutions

`prospect_agent`
- builds prospect intelligence summaries using public and internal authorized data

`territory_agent`
- evaluates neighborhoods, municipalities, and service areas using Census, property, and institutional signals

`property_agent`
- answers parcel, ownership, lien, and document-history questions

`institution_agent`
- maps organizations, facilities, boards, partnerships, and influence networks

`expansion_agent`
- creates market and expansion briefs for target geographies

`watchlist_agent`
- monitors changes in records, values, entities, and relevant public signals

`briefing_agent`
- turns all retrieved evidence into concise, executive-ready memos with citations

### Agent Operating Rules

- structured search before vector retrieval
- vector retrieval before OCR/original fetch
- every output must preserve provenance
- facts must be separated from inference
- sensitive outputs require human review

## Cleveland Clinic Use Cases

The first internal customer should be Cleveland Clinic South Florida strategy and fundraising work.

### Immediate Use Cases

- identify high-potential fundraising territories in Broward
- build prospect research briefs with property and organization context
- map institutional influence and partnership networks
- assess service-area growth and demographics
- generate municipality or corridor-specific expansion intelligence
- monitor key geographic zones for changes relevant to healthcare planning

### Example Questions the System Must Answer

- Which Broward geographies combine wealth concentration, population growth, and healthcare expansion relevance?
- Which individuals, families, entities, or trusts hold significant real-estate footprints in a target service area?
- What hospitals, nonprofits, employers, and civic actors dominate a target geography?
- What recent real-estate and development changes suggest rising philanthropic or market potential?
- Build a briefing for leadership on a target corridor or municipality.

## Government and Funder Use Cases

Once the Broward model works internally, it can be adapted for external clients.

### Governments

- neighborhood intelligence
- development and housing analysis
- institutional landscape mapping
- parcel and ownership pattern analysis
- public decision briefing support

### Funders and Nonprofits

- prospect and donor territory intelligence
- campaign planning
- place-based funding strategy
- grantee and institution landscape analysis
- board and executive research support

### Hospitals and Health Systems

- expansion strategy
- philanthropic capacity mapping
- service-area intelligence
- partnership identification
- executive briefings

## Compliance and Ethics Guardrails

This system becomes commercially valuable only if it remains legally and ethically defensible.

### Non-Negotiable Rules

- do not scrape restricted sites or violate terms of service
- do not present speculative wealth or identity inference as fact
- do not generate covert household dossiers
- do not mix sensitive internal CRM data into general public outputs without authorization controls
- preserve fact versus inference boundaries in every workflow
- retain source citations for every material claim

### Approved Categories

- public records
- official websites
- official reports
- public news sources
- Census and other official government data
- licensed data where contractually permitted

### Restricted or High-Risk Categories

- LinkedIn scraping
- opaque personal profiling
- consumer-report-style eligibility scoring
- unsupported "social net worth" claims
- hidden surveillance-style monitoring of private individuals

### Human Review Requirement

Any output involving:

- donor prioritization
- institutional strategy
- reputational conclusions
- major prospect briefing
- inferred network relationships

should be reviewed by a human before operational use.

## System Architecture in LibreFang

LibreFang already provides the right architectural foundation for this project.

### Why LibreFang Fits

- multi-tenant architecture
- agent orchestration
- memory systems
- tool routing
- vector integration
- API surface for internal apps and dashboards

### Recommended Internal Components

- ingestion workers for county and enrichment sources
- normalization pipeline for records and entities
- graph-backed relational store for truth
- Supabase or Postgres-backed vector layer for semantic retrieval
- document store for originals and OCR artifacts
- internal APIs for search, brief generation, watchlists, and research actions
- dashboard and workspace UI for analysts and executives

### Suggested Logical Services

- `source_ingest_service`
- `entity_resolution_service`
- `county_graph_service`
- `retrieval_service`
- `briefing_service`
- `watchlist_service`
- `document_evidence_service`
- `crm_sync_service`

## 90-Day Execution Plan

The first 90 days should focus on making the system useful, not perfect.

### Phase 1: Foundation, Weeks 1-3

- ingest Broward recorder yearly text exports
- stand up daily recorder text sync
- ingest Broward appraiser or assessor data
- ingest Census tract and block-group datasets
- define canonical schema
- build baseline search indexes

Deliverable:
- a working Broward backbone with properties, parcels, documents, and parties linked

### Phase 2: Intelligence, Weeks 4-7

- implement entity resolution for people, organizations, trusts, and LLCs
- link parcels to addresses and Census geographies
- create event timelines for ownership, mortgages, liens, and litigation
- create synthesized summaries for embedding
- stand up initial agent workflows

Deliverable:
- a usable intelligence graph rather than a raw warehouse

### Phase 3: Decision Support, Weeks 8-10

- create Cleveland Clinic-specific briefing templates
- define target corridors and strategic geographies
- build territory and expansion workflows
- stand up watchlists and change alerts
- test research outputs against real leadership questions

Deliverable:
- source-backed intelligence briefs and monitoring for real internal decisions

### Phase 4: Productization, Weeks 11-13

- package workflows into reusable modules
- define tenant and permission boundaries
- create a repeatable external service model
- document compliance and review workflow
- prepare pilot offerings for governments and funders

Deliverable:
- a viable external-facing service architecture based on the same county brain

## Build Priorities

The build order matters.

### Priority 1

- recorder text ingestion
- parcel and address normalization
- Census and geography linking
- deterministic search and filtering

### Priority 2

- entity resolution
- synthesized summaries and embeddings
- agent briefing workflows
- watchlists

### Priority 3

- original document OCR for selected records
- institutional and board mapping
- CRM sync
- external tenant packaging

## First Five Workflows

The first live workflows should be narrow and high-value.

### Workflow 1: Property and Ownership Timeline

Input:
- parcel, address, name, or recording number

Output:
- linked document history
- party history
- ownership and mortgage timeline
- citations to source records

### Workflow 2: Prospect Brief

Input:
- person, household, trust, or entity

Output:
- property footprint
- organization affiliations
- relevant public context
- source-backed research brief

### Workflow 3: Geography Opportunity Brief

Input:
- municipality, ZIP code, corridor, or Census tract set

Output:
- demographic profile
- institutional landscape
- property and development signals
- philanthropic potential narrative

### Workflow 4: Expansion Memo

Input:
- target geography or corridor

Output:
- market context
- growth signals
- institutional landscape
- strategic opportunities and constraints

### Workflow 5: Watchlist Alert

Input:
- named prospects, entities, geographies, or parcel sets

Output:
- daily or weekly material changes
- concise impact summary
- links to source evidence

## Measures of Success

The project should be evaluated on business usefulness, not just data volume.

### Operational Success Metrics

- time to answer a research question
- number of usable briefs produced per week
- recall and precision of entity resolution
- percentage of outputs with full citations
- number of internal planning decisions informed by the system

### Strategic Success Metrics

- improved quality of fundraising target selection
- improved quality of geographic expansion planning
- faster executive briefing preparation
- successful delivery of pilot external intelligence engagements

## Immediate Next Steps

The next actions should be concrete and sequenced.

1. Confirm the Phase 1 source list for Broward.
2. Define the canonical schema and identifiers.
3. List the first five Cleveland Clinic questions the system must answer.
4. Implement recorder text ingestion and appraiser ingestion.
5. Build one end-to-end briefing workflow and validate it with a real use case.

## Conclusion

Broward County is a strong proving ground for building a county-scale intelligence system inside LibreFang. The recorder data provides a powerful base layer, especially when combined with appraiser, Census, institutional, and public-context datasets. LibreFang can serve as the orchestration, retrieval, and agent layer on top of that graph.

The main strategic decision is to treat this as an intelligence platform, not a raw data product. If executed well, the Broward County Brain becomes:

- an internal strategic asset for Cleveland Clinic fundraising and expansion
- a reusable county intelligence framework
- the foundation for a high-value services and software business
