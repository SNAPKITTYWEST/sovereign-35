"""
FRANKENSTEIN HANDS — Sovereign Knowledge Retrieval Sidecar
Port 5433

Pipeline:
  query
    → spaCy + NLTK   (NLP parse: entities, POS, intent, key concepts)
    → FAISS          (vector similarity → top-K corpus chunks)
    → Neo4j          (graph traversal → expand related nodes)
    → RDF / rdflib   (semantic triples for extracted entities)
    → NetworkX       (rank by PageRank / betweenness centrality)
    → tagged context → Frankenstein Brain

Each layer degrades gracefully if its backend is offline.
Every retrieval is WORM-tagged with source metadata.

SnapKitty Collective 2026 · Apache 2.0 · Evidence or Silence
"""

import hashlib, json, os, time
from datetime import datetime
from typing import Optional
from fastapi import FastAPI
from pydantic import BaseModel

# ── Optional heavy deps — degrade gracefully ──────────────────────────────────
try:
    import spacy
    nlp = spacy.load("en_core_web_sm")
    SPACY_OK = True
except Exception:
    SPACY_OK = False

try:
    import nltk
    from nltk.corpus import wordnet, stopwords
    from nltk.tokenize import word_tokenize
    nltk.download("punkt",       quiet=True)
    nltk.download("wordnet",     quiet=True)
    nltk.download("stopwords",   quiet=True)
    nltk.download("averaged_perceptron_tagger", quiet=True)
    NLTK_OK = True
    STOPS = set(stopwords.words("english"))
except Exception:
    NLTK_OK = False
    STOPS   = set()

try:
    import faiss
    import numpy as np
    FAISS_OK = True
except Exception:
    FAISS_OK = False

try:
    from neo4j import GraphDatabase
    NEO4J_URI  = os.getenv("NEO4J_URI",  "bolt://localhost:7687")
    NEO4J_USER = os.getenv("NEO4J_USER", "neo4j")
    NEO4J_PASS = os.getenv("NEO4J_PASS", "sovereign")
    neo4j_driver = GraphDatabase.driver(NEO4J_URI, auth=(NEO4J_USER, NEO4J_PASS))
    NEO4J_OK = True
except Exception:
    NEO4J_OK = False

try:
    from rdflib import Graph as RDFGraph, URIRef, Literal, Namespace
    from rdflib.namespace import RDF, RDFS, OWL
    rdf_g = RDFGraph()
    SKC = Namespace("https://snapkitty.io/ontology#")
    rdf_g.bind("skc", SKC)
    RDF_OK = True
except Exception:
    RDF_OK = False

try:
    import networkx as nx
    NETX_OK = True
except Exception:
    NETX_OK = False

try:
    from sentence_transformers import SentenceTransformer
    embed_model = SentenceTransformer("all-MiniLM-L6-v2")
    EMBED_OK = True
except Exception:
    EMBED_OK = False

# ── In-memory FAISS index (loaded from corpus on startup) ─────────────────────
faiss_index = None
faiss_texts: list[str] = []
faiss_meta:  list[dict] = []

# ── NetworkX knowledge graph (built from Neo4j + RDF on startup) ──────────────
nx_graph = nx.DiGraph() if NETX_OK else None

# ── WORM chain ────────────────────────────────────────────────────────────────
worm_prev  = "HANDS_GENESIS"
worm_count = 0

def worm_seal(event: str) -> str:
    global worm_prev, worm_count
    msg  = f"{worm_prev}|{event}|{int(time.time()*1000)}"
    h    = hashlib.sha256(msg.encode()).hexdigest()
    worm_prev  = h
    worm_count += 1
    return h[:16]

# ── FastAPI app ───────────────────────────────────────────────────────────────
app = FastAPI(title="FRANKENSTEIN HANDS", version="1.0.0")

class RetrieveRequest(BaseModel):
    query: str
    k:     int = 5

class IndexRequest(BaseModel):
    texts: list[str]
    metas: list[dict] = []

class NeoIngestRequest(BaseModel):
    nodes: list[dict]   # [{"id": "...", "label": "...", "props": {...}}]
    edges: list[dict]   # [{"from": "...", "to": "...", "rel": "..."}]

# ── NLP PARSE — spaCy + NLTK ──────────────────────────────────────────────────
def nlp_parse(query: str) -> dict:
    result = {"entities": [], "concepts": [], "pos_tags": [], "wordnet": [], "intent": "query"}

    if SPACY_OK:
        doc = nlp(query)
        result["entities"] = [
            {"text": ent.text, "label": ent.label_, "start": ent.start_char, "end": ent.end_char}
            for ent in doc.ents
        ]
        result["concepts"] = [
            tok.lemma_ for tok in doc
            if tok.pos_ in ("NOUN", "VERB", "PROPN") and not tok.is_stop and len(tok.text) > 2
        ]
        result["pos_tags"] = [{"text": tok.text, "pos": tok.pos_} for tok in doc]

        # Detect intent from dependency root
        for tok in doc:
            if tok.dep_ == "ROOT":
                if tok.pos_ == "VERB":
                    result["intent"] = tok.lemma_
                break

    if NLTK_OK:
        tokens  = word_tokenize(query)
        content = [t for t in tokens if t.lower() not in STOPS and t.isalpha()]
        result["nltk_tokens"] = content

        # WordNet synonyms for key concepts
        wn_hits = []
        for word in content[:4]:
            syns = wordnet.synsets(word)
            if syns:
                wn_hits.append({
                    "word":       word,
                    "definition": syns[0].definition(),
                    "examples":   syns[0].examples()[:1],
                    "hypernyms":  [h.name() for h in syns[0].hypernyms()[:2]],
                })
        result["wordnet"] = wn_hits

    return result

# ── FAISS RETRIEVE ────────────────────────────────────────────────────────────
def faiss_retrieve(query: str, k: int) -> list[dict]:
    if not FAISS_OK or not EMBED_OK or faiss_index is None:
        return [{"text": "FAISS_OFFLINE — no index loaded", "score": 0.0, "meta": {}}]

    vec = embed_model.encode([query], normalize_embeddings=True).astype("float32")
    scores, idxs = faiss_index.search(vec, min(k, len(faiss_texts)))
    return [
        {
            "text":  faiss_texts[i],
            "score": float(scores[0][n]),
            "meta":  faiss_meta[i] if i < len(faiss_meta) else {},
        }
        for n, i in enumerate(idxs[0]) if i >= 0
    ]

# ── NEO4J EXPAND ─────────────────────────────────────────────────────────────
def neo4j_expand(concepts: list[str], k: int) -> list[dict]:
    if not NEO4J_OK or not concepts:
        return [{"node": "NEO4J_OFFLINE", "rels": []}]
    try:
        with neo4j_driver.session() as s:
            results = []
            for concept in concepts[:3]:
                q = (
                    "MATCH (n)-[r]->(m) "
                    "WHERE toLower(n.name) CONTAINS toLower($c) "
                    "RETURN n.name AS src, type(r) AS rel, m.name AS tgt, m.description AS desc "
                    "LIMIT $k"
                )
                rows = s.run(q, c=concept, k=k).data()
                results.extend([{
                    "node": r["src"], "rel": r["rel"],
                    "target": r["tgt"], "desc": r.get("desc","")
                } for r in rows])
            return results or [{"node": "NO_MATCH", "concept": concepts}]
    except Exception as e:
        return [{"node": f"NEO4J_ERR: {e}"}]

# ── RDF TRIPLES ───────────────────────────────────────────────────────────────
def rdf_triples(entities: list[str]) -> list[dict]:
    if not RDF_OK or not entities:
        return []
    results = []
    for ent in entities[:4]:
        subj = URIRef(f"https://snapkitty.io/ontology#{ent.replace(' ','_')}")
        for _, pred, obj in rdf_g.triples((subj, None, None)):
            results.append({"subject": ent, "predicate": str(pred), "object": str(obj)})
    return results or [{"rdf": "NO_TRIPLES_YET — load an ontology via /ingest/rdf"}]

# ── NETWORKX RANK ─────────────────────────────────────────────────────────────
def netx_rank(faiss_hits: list[dict], neo4j_hits: list[dict]) -> list[dict]:
    if not NETX_OK:
        return faiss_hits

    # Build mini graph from retrieved hits
    G = nx.DiGraph()
    for h in faiss_hits:
        G.add_node(h["text"][:64], score=h["score"], type="corpus")
    for h in neo4j_hits:
        src = h.get("node","?")
        tgt = h.get("target","?")
        if src and tgt and src != "NEO4J_OFFLINE":
            G.add_edge(src, tgt, rel=h.get("rel","RELATED"))

    if len(G.nodes) == 0:
        return faiss_hits

    try:
        pr = nx.pagerank(G, alpha=0.85)
    except Exception:
        pr = {}

    ranked = sorted(faiss_hits, key=lambda h: pr.get(h["text"][:64], h["score"]), reverse=True)
    return ranked

# ── Task pool helpers (run sync functions in thread pool) ─────────────────────
import asyncio
from concurrent.futures import ThreadPoolExecutor

_pool = ThreadPoolExecutor(max_workers=8, thread_name_prefix="hands")

async def _run(fn, *args):
    loop = asyncio.get_running_loop()
    return await loop.run_in_executor(_pool, fn, *args)

async def _safe(label: str, coro):
    """Run a coroutine with error isolation — never lets one layer kill the rest."""
    try:
        return label, await coro
    except asyncio.TimeoutError:
        return label, {"error": f"{label}_TIMEOUT"}
    except Exception as e:
        return label, {"error": f"{label}_ERR: {type(e).__name__}: {e}"}

# ── /retrieve — main endpoint (TaskGroup: all layers fire concurrently) ────────
@app.post("/retrieve")
async def retrieve(req: RetrieveRequest):
    t0 = time.time()

    # Phase 1: NLP parse (must complete before graph layers need concepts)
    parse = await _run(nlp_parse, req.query)
    concepts  = parse.get("concepts", [])
    ent_texts = [e["text"] for e in parse.get("entities", [])]

    # Phase 2: Fire FAISS + Neo4j + RDF concurrently via TaskGroup
    # Each layer is isolated — failure in one does NOT cancel others
    layer_results = {}

    async def _faiss():
        return await asyncio.wait_for(_run(faiss_retrieve, req.query, req.k), timeout=5.0)

    async def _neo4j():
        return await asyncio.wait_for(_run(neo4j_expand, concepts, req.k), timeout=8.0)

    async def _rdf():
        return await asyncio.wait_for(_run(rdf_triples, ent_texts), timeout=3.0)

    # asyncio.gather with return_exceptions=True → error in one doesn't cancel others
    faiss_r, neo4j_r, rdf_r = await asyncio.gather(
        _faiss(), _neo4j(), _rdf(),
        return_exceptions=True
    )

    # Normalize exceptions into error dicts
    def safe_result(r, default):
        if isinstance(r, Exception):
            return [{"error": f"{type(r).__name__}: {r}"}]
        return r if r else default

    chunks  = safe_result(faiss_r, [{"text": "FAISS_OFFLINE", "score": 0, "meta": {}}])
    neo4j   = safe_result(neo4j_r, [{"node": "NEO4J_OFFLINE"}])
    triples = safe_result(rdf_r,   [])

    # Phase 3: NetworkX rank (uses outputs from phase 2)
    ranked = await _run(netx_rank, chunks, neo4j)

    # WORM seal
    seal = worm_seal(f"RETRIEVE|{req.query[:32]}")

    # Build context string for Brain (formatted for easy consumption)
    context_lines = []
    for i, r in enumerate(ranked[:req.k]):
        text = r.get("text","")
        if text and not text.startswith("FAISS_OFFLINE"):
            context_lines.append(f"[{i+1}] (score={r.get('score',0):.3f}) {text}")
    for g in neo4j[:3]:
        if "node" in g and g["node"] not in ("NEO4J_OFFLINE","NO_MATCH"):
            context_lines.append(f"[GRAPH] {g.get('node','')} --{g.get('rel','')}→ {g.get('target','')}: {g.get('desc','')}")
    for t in triples[:2]:
        context_lines.append(f"[RDF] {t.get('subject','')} {t.get('predicate','').split('#')[-1]} {t.get('object','')}")

    results = [{"text": r["text"], "score": r.get("score",0), "meta": r.get("meta",{})} for r in ranked]

    return {
        "query":    req.query,
        "results":  results,
        "context":  "\n".join(context_lines) if context_lines else "NO_CONTEXT",
        "nlp":      parse,
        "graph":    neo4j[:5],
        "rdf":      triples[:5],
        "layers": {
            "spacy":      SPACY_OK,
            "nltk":       NLTK_OK,
            "faiss":      FAISS_OK and faiss_index is not None,
            "neo4j":      NEO4J_OK,
            "rdf":        RDF_OK,
            "networkx":   NETX_OK,
            "embed":      EMBED_OK,
        },
        "worm": seal,
        "ms":   round((time.time() - t0) * 1000, 1),
    }

# ── /index — load corpus into FAISS ──────────────────────────────────────────
@app.post("/index")
async def index_corpus(req: IndexRequest):
    global faiss_index, faiss_texts, faiss_meta
    if not FAISS_OK or not EMBED_OK:
        return {"error": "FAISS or sentence-transformers not installed"}

    vecs = embed_model.encode(req.texts, normalize_embeddings=True).astype("float32")
    dim  = vecs.shape[1]
    faiss_index = faiss.IndexFlatIP(dim)  # Inner product = cosine (normalized vecs)
    faiss_index.add(vecs)
    faiss_texts = req.texts
    faiss_meta  = req.metas or [{} for _ in req.texts]

    seal = worm_seal(f"INDEX|{len(req.texts)}_docs")
    return {"indexed": len(req.texts), "dim": dim, "worm": seal}

# ── /ingest/rdf — load RDF triples ───────────────────────────────────────────
@app.post("/ingest/rdf")
async def ingest_rdf(payload: dict):
    if not RDF_OK:
        return {"error": "rdflib not installed"}
    triples_added = 0
    for triple in payload.get("triples", []):
        s = URIRef(triple["subject"])
        p = URIRef(triple["predicate"])
        o = Literal(triple["object"]) if triple.get("literal") else URIRef(triple["object"])
        rdf_g.add((s, p, o))
        triples_added += 1
    return {"added": triples_added, "total": len(rdf_g)}

# ── /ingest/graph — load into Neo4j + NetworkX ────────────────────────────────
@app.post("/ingest/graph")
async def ingest_graph(req: NeoIngestRequest):
    results = {"neo4j": "offline", "networkx": 0}
    if NEO4J_OK:
        try:
            with neo4j_driver.session() as s:
                for node in req.nodes:
                    s.run(
                        "MERGE (n:Entity {id: $id}) SET n.name=$name, n += $props",
                        id=node["id"], name=node.get("label",""), props=node.get("props",{})
                    )
                for edge in req.edges:
                    s.run(
                        f"MATCH (a:Entity {{id:$f}}),(b:Entity {{id:$t}}) MERGE (a)-[:{edge['rel']}]->(b)",
                        f=edge["from"], t=edge["to"]
                    )
                results["neo4j"] = f"ok — {len(req.nodes)} nodes, {len(req.edges)} edges"
        except Exception as e:
            results["neo4j"] = f"error: {e}"

    if NETX_OK:
        for node in req.nodes:
            nx_graph.add_node(node["id"], **node.get("props",{}))
        for edge in req.edges:
            nx_graph.add_edge(edge["from"], edge["to"], rel=edge["rel"])
        results["networkx"] = nx_graph.number_of_nodes()

    return results

# ── /health ────────────────────────────────────────────────────────────────────
@app.get("/health")
async def health():
    return {
        "ok": True,
        "service": "frankenstein-hands",
        "version": "1.0.0",
        "worm_seals": worm_count,
        "layers": {
            "spacy":    SPACY_OK,
            "nltk":     NLTK_OK,
            "faiss":    FAISS_OK,
            "neo4j":    NEO4J_OK,
            "rdf":      RDF_OK,
            "networkx": NETX_OK,
            "embed":    EMBED_OK,
            "index_loaded": faiss_index is not None,
        }
    }

if __name__ == "__main__":
    import uvicorn
    print("⬡ FRANKENSTEIN HANDS → http://localhost:5433")
    print("  FAISS + Neo4j + RDF + NetworkX + spaCy + NLTK")
    uvicorn.run(app, host="0.0.0.0", port=5433)
