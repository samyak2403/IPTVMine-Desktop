use scraper::{ElementRef, Html, Selector};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

#[derive(Debug, Default)]
pub struct HtmlDomStore {
    next_doc_id: usize,
    docs: HashMap<usize, Html>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomNodePayload {
    pub doc_id: usize,
    pub node_id: usize,
    pub text: String,
    pub html: String,
    pub attrs: HashMap<String, String>,
    pub tag: String,
}

impl HtmlDomStore {
    pub fn insert(&mut self, html_str: &str) -> usize {
        self.next_doc_id += 1;
        let id = self.next_doc_id;
        // Keep bounded to avoid unbounded memory growth during long sessions
        if self.docs.len() >= 48 {
            if let Some(&oldest) = self.docs.keys().min() {
                self.docs.remove(&oldest);
            }
        }
        self.docs.insert(id, Html::parse_document(html_str));
        id
    }

    pub fn select(&self, doc_id: usize, scope_node_id: Option<usize>, css: &str) -> Vec<DomNodePayload> {
        let Some(doc) = self.docs.get(&doc_id) else {
            return Vec::new();
        };
        let scope_el = scope_node_id.and_then(|nid| find_element(doc, nid));
        if scope_node_id.is_some() && scope_el.is_none() {
            return Vec::new();
        }
        let scope_nid = scope_el.map(|el| node_index(&el));

        let mut results = Vec::new();
        let mut seen_ids = HashSet::new();

        for part in split_selector_parts(css) {
            let spec = parse_jquery_pseudo(&part);
            let effective_css = if spec.base_css.is_empty() { "*" } else { spec.base_css.as_str() };
            let Ok(selector) = Selector::parse(effective_css) else {
                continue;
            };

            let mut matched: Vec<ElementRef<'_>> = if let Some(el) = scope_el {
                el.select(&selector)
                    .filter(|child| Some(node_index(child)) != scope_nid)
                    .collect()
            } else {
                doc.select(&selector).collect()
            };

            apply_pseudo_filters(&mut matched, &spec);
            for el in matched {
                push_unique_payload(&mut results, &mut seen_ids, doc_id, el);
            }
        }

        results
    }

    pub fn traverse(
        &self,
        doc_id: usize,
        node_ids: &[usize],
        axis: &str,
        css: Option<&str>,
    ) -> Vec<DomNodePayload> {
        let Some(doc) = self.docs.get(&doc_id) else {
            return Vec::new();
        };
        let filter_css = css.map(str::trim).filter(|s| !s.is_empty());
        let mut results = Vec::new();
        let mut seen = HashSet::new();

        for &nid in node_ids {
            let Some(el) = find_element(doc, nid) else {
                continue;
            };
            match axis {
                "children" => {
                    for child in el.child_elements() {
                        if matches_optional(doc, child, filter_css) {
                            push_unique_payload(&mut results, &mut seen, doc_id, child);
                        }
                    }
                }
                "parent" => {
                    if let Some(parent) = el.parent().and_then(ElementRef::wrap) {
                        if matches_optional(doc, parent, filter_css) {
                            push_unique_payload(&mut results, &mut seen, doc_id, parent);
                        }
                    }
                }
                "parents" => {
                    let mut cur = el.parent();
                    while let Some(node) = cur {
                        if let Some(pel) = ElementRef::wrap(node) {
                            if matches_optional(doc, pel, filter_css) {
                                push_unique_payload(&mut results, &mut seen, doc_id, pel);
                            }
                        }
                        cur = node.parent();
                    }
                }
                "closest" => {
                    if let Some(target_css) = filter_css {
                        let mut cur = Some(*el);
                        while let Some(node) = cur {
                            if let Some(pel) = ElementRef::wrap(node) {
                                if element_matches_selector(doc, pel, target_css) {
                                    push_unique_payload(&mut results, &mut seen, doc_id, pel);
                                    break;
                                }
                            }
                            cur = node.parent();
                        }
                    }
                }
                "next" => {
                    if let Some(hit) = el
                        .next_siblings()
                        .filter_map(ElementRef::wrap)
                        .find(|sib| matches_optional(doc, *sib, filter_css))
                    {
                        push_unique_payload(&mut results, &mut seen, doc_id, hit);
                    }
                }
                "nextAll" => {
                    for sib in el.next_siblings().filter_map(ElementRef::wrap) {
                        if matches_optional(doc, sib, filter_css) {
                            push_unique_payload(&mut results, &mut seen, doc_id, sib);
                        }
                    }
                }
                "nextUntil" => {
                    for sib in el.next_siblings().filter_map(ElementRef::wrap) {
                        if let Some(stop_css) = filter_css {
                            if element_matches_selector(doc, sib, stop_css) {
                                break;
                            }
                        }
                        push_unique_payload(&mut results, &mut seen, doc_id, sib);
                    }
                }
                "prev" => {
                    if let Some(hit) = el
                        .prev_siblings()
                        .filter_map(ElementRef::wrap)
                        .find(|sib| matches_optional(doc, *sib, filter_css))
                    {
                        push_unique_payload(&mut results, &mut seen, doc_id, hit);
                    }
                }
                "prevAll" => {
                    for sib in el.prev_siblings().filter_map(ElementRef::wrap) {
                        if matches_optional(doc, sib, filter_css) {
                            push_unique_payload(&mut results, &mut seen, doc_id, sib);
                        }
                    }
                }
                "siblings" => {
                    if let Some(parent) = el.parent().and_then(ElementRef::wrap) {
                        for sib in parent.child_elements() {
                            if node_index(&sib) != nid && matches_optional(doc, sib, filter_css) {
                                push_unique_payload(&mut results, &mut seen, doc_id, sib);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        results
    }

    pub fn filter_nodes(
        &self,
        doc_id: usize,
        node_ids: &[usize],
        css: &str,
        negate: bool,
    ) -> Vec<DomNodePayload> {
        let Some(doc) = self.docs.get(&doc_id) else {
            return Vec::new();
        };
        let mut results = Vec::new();
        let mut seen = HashSet::new();
        for &nid in node_ids {
            if let Some(el) = find_element(doc, nid) {
                let matched = element_matches_selector(doc, el, css);
                if matched ^ negate {
                    push_unique_payload(&mut results, &mut seen, doc_id, el);
                }
            }
        }
        results
    }

    pub fn remove(&mut self, doc_id: usize, node_ids: &[usize]) {
        let Some(doc) = self.docs.get_mut(&doc_id) else {
            return;
        };
        let root_id = doc.tree.root().id();
        let raw_ids: Vec<_> = node_ids
            .iter()
            .filter_map(|&nid| find_raw_node_id(doc, nid))
            .filter(|id| *id != root_id)
            .collect();
        for raw_id in raw_ids {
            if let Some(mut node) = doc.tree.get_mut(raw_id) {
                node.detach();
            }
        }
    }

    pub fn clone_nodes(&mut self, doc_id: usize, node_ids: &[usize]) -> (usize, Vec<DomNodePayload>) {
        let html_chunk = {
            let Some(doc) = self.docs.get(&doc_id) else {
                return (doc_id, Vec::new());
            };
            node_ids
                .iter()
                .filter_map(|&nid| find_element(doc, nid))
                .map(|el| el.html())
                .collect::<Vec<_>>()
                .join("")
        };
        if html_chunk.is_empty() {
            return (doc_id, Vec::new());
        }
        let cloned_doc_id = self.insert(&html_chunk);
        let cloned_nodes = self.select(cloned_doc_id, None, "body > *");
        (cloned_doc_id, cloned_nodes)
    }

    pub fn texts(&self, doc_id: usize, node_ids: &[usize]) -> String {
        let Some(doc) = self.docs.get(&doc_id) else {
            return String::new();
        };
        node_ids
            .iter()
            .filter_map(|&nid| find_element(doc, nid))
            .map(|el| el.text().collect::<Vec<_>>().join(""))
            .collect::<Vec<_>>()
            .join("")
    }

    pub fn inner_html(&self, doc_id: usize, node_id: usize) -> String {
        let Some(doc) = self.docs.get(&doc_id) else {
            return String::new();
        };
        find_element(doc, node_id)
            .map(|el| el.inner_html())
            .unwrap_or_default()
    }

    pub fn root_html(&self, doc_id: usize) -> String {
        self.docs
            .get(&doc_id)
            .map(|d| d.html())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy)]
enum PosFilter {
    First,
    Last,
    Eq(usize),
}

struct SelectorSpec {
    base_css: String,
    contains: Option<String>,
    not_contains: Option<String>,
    pos: Option<PosFilter>,
}

fn find_raw_node_id(doc: &Html, nid: usize) -> Option<ego_tree::NodeId> {
    doc.tree
        .nodes()
        .find(|n| node_index_raw(n.id()) == nid)
        .map(|n| n.id())
}

fn find_element(doc: &Html, nid: usize) -> Option<ElementRef<'_>> {
    let raw_id = find_raw_node_id(doc, nid)?;
    let node = doc.tree.get(raw_id)?;
    ElementRef::wrap(node)
}

fn push_unique_payload(
    results: &mut Vec<DomNodePayload>,
    seen: &mut HashSet<usize>,
    doc_id: usize,
    el: ElementRef<'_>,
) {
    let nid = node_index(&el);
    if seen.insert(nid) {
        results.push(payload_from_element(doc_id, el));
    }
}

fn matches_optional(doc: &Html, el: ElementRef<'_>, css: Option<&str>) -> bool {
    css.map(|s| element_matches_selector(doc, el, s))
        .unwrap_or(true)
}

fn element_matches_selector(doc: &Html, el: ElementRef<'_>, css: &str) -> bool {
    let nid = node_index(&el);
    for part in split_selector_parts(css) {
        let spec = parse_jquery_pseudo(&part);
        let effective_css = if spec.base_css.is_empty() { "*" } else { spec.base_css.as_str() };
        let Ok(selector) = Selector::parse(effective_css) else {
            continue;
        };
        if !selector.matches(&el) {
            continue;
        }
        let txt: String = el.text().collect();
        let txt_low = txt.to_ascii_lowercase();
        if let Some(needle) = &spec.contains {
            if !txt_low.contains(&needle.to_ascii_lowercase()) {
                continue;
            }
        }
        if let Some(excl) = &spec.not_contains {
            if txt_low.contains(&excl.to_ascii_lowercase()) {
                continue;
            }
        }
        if let Some(pos) = spec.pos {
            let mut all: Vec<ElementRef<'_>> = doc.select(&selector).collect();
            apply_pseudo_filters(
                &mut all,
                &SelectorSpec {
                    base_css: spec.base_css.clone(),
                    contains: spec.contains.clone(),
                    not_contains: spec.not_contains.clone(),
                    pos: Some(pos),
                },
            );
            if !all.iter().any(|m| node_index(m) == nid) {
                continue;
            }
        }
        return true;
    }
    false
}

fn apply_pseudo_filters(matched: &mut Vec<ElementRef<'_>>, spec: &SelectorSpec) {
    if let Some(needle) = &spec.contains {
        let needle_low = needle.to_ascii_lowercase();
        matched.retain(|el| {
            let txt: String = el.text().collect();
            txt.to_ascii_lowercase().contains(&needle_low)
        });
    }
    if let Some(excl) = &spec.not_contains {
        let excl_low = excl.to_ascii_lowercase();
        matched.retain(|el| {
            let txt: String = el.text().collect();
            !txt.to_ascii_lowercase().contains(&excl_low)
        });
    }
    match spec.pos {
        Some(PosFilter::First) => matched.truncate(1),
        Some(PosFilter::Last) => {
            if let Some(last) = matched.pop() {
                *matched = vec![last];
            }
        }
        Some(PosFilter::Eq(idx)) => {
            *matched = matched.get(idx).copied().into_iter().collect();
        }
        None => {}
    }
}

fn split_selector_parts(css: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut depth = 0usize;
    for ch in css.chars() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
            cur.push(ch);
        } else {
            match ch {
                '"' | '\'' => {
                    quote = Some(ch);
                    cur.push(ch);
                }
                '(' | '[' => {
                    depth += 1;
                    cur.push(ch);
                }
                ')' | ']' => {
                    depth = depth.saturating_sub(1);
                    cur.push(ch);
                }
                ',' if depth == 0 => {
                    let t = cur.trim();
                    if !t.is_empty() {
                        parts.push(t.to_string());
                    }
                    cur.clear();
                }
                _ => cur.push(ch),
            }
        }
    }
    let t = cur.trim();
    if !t.is_empty() {
        parts.push(t.to_string());
    }
    parts
}

fn strip_pseudo_arg(s: &mut String, needle: &str) -> Option<String> {
    let idx = s.to_ascii_lowercase().find(needle)?;
    let start = idx + needle.len();
    let mut depth = 1usize;
    let mut quote: Option<char> = None;
    let mut end_rel = None;
    for (off, ch) in s[start..].char_indices() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
        } else if ch == '"' || ch == '\'' {
            quote = Some(ch);
        } else if ch == '(' {
            depth += 1;
        } else if ch == ')' {
            depth -= 1;
            if depth == 0 {
                end_rel = Some(off);
                break;
            }
        }
    }
    let end = end_rel?;
    let inner = s[start..start + end]
        .trim()
        .trim_matches(|c| c == '\'' || c == '"')
        .to_string();
    s.replace_range(idx..start + end + 1, "");
    Some(inner)
}

fn parse_jquery_pseudo(part: &str) -> SelectorSpec {
    let mut s = part.trim().to_string();
    let mut not_contains = None;
    if let Some(inner) = strip_pseudo_arg(&mut s, ":not(:contains(") {
        // Note: strip_pseudo_arg matched outer `:not(`'s matching `)` if balanced, or inner `)`
        let trimmed = inner.trim_end_matches(')').trim().trim_matches(|c| c == '\'' || c == '"').to_string();
        if s.starts_with(')') {
            s.remove(0);
        } else if let Some(rest_idx) = s.find(')') {
            if s[..rest_idx].trim().is_empty() {
                s.replace_range(..rest_idx + 1, "");
            }
        }
        not_contains = Some(trimmed);
    }
    let contains = strip_pseudo_arg(&mut s, ":contains(");
    let mut pos = None;

    if let Some(idx) = s.to_ascii_lowercase().find(":first") {
        pos = Some(PosFilter::First);
        s.replace_range(idx..idx + 6, "");
    } else if let Some(idx) = s.to_ascii_lowercase().find(":last") {
        pos = Some(PosFilter::Last);
        s.replace_range(idx..idx + 5, "");
    } else if let Some(num) = strip_pseudo_arg(&mut s, ":eq(") {
        if let Ok(n) = num.trim().parse::<usize>() {
            pos = Some(PosFilter::Eq(n));
        }
    }

    SelectorSpec {
        base_css: s.trim().to_string(),
        contains,
        not_contains,
        pos,
    }
}

fn node_index_raw(id: ego_tree::NodeId) -> usize {
    format!("{id:?}")
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse::<usize>()
        .unwrap_or(0)
}

fn node_index(el: &ElementRef<'_>) -> usize {
    node_index_raw(el.id())
}

fn payload_from_element(doc_id: usize, el: ElementRef<'_>) -> DomNodePayload {
    let mut attrs = HashMap::new();
    for (k, v) in el.value().attrs() {
        attrs.insert(k.to_string(), v.to_string());
    }
    DomNodePayload {
        doc_id,
        node_id: node_index(&el),
        text: el.text().collect::<Vec<_>>().join(""),
        html: el.inner_html(),
        attrs,
        tag: el.value().name().to_string(),
    }
}

pub type SharedDomStore = std::sync::Arc<Mutex<HtmlDomStore>>;

/// JS polyfill implementing `cheerio.load(html)` backed by native Rust `scraper` host functions.
pub const CHEERIO_JS_SHIM: &str = r#"
(function() {
  function enrichNode(n) {
    if (!n || typeof n !== "object") return n;
    if (!n.attribs) n.attribs = n.attrs || {};
    if (!n.name) n.name = n.tag || "";
    if (!n.type) n.type = "tag";
    return n;
  }

  function makeWrapper(docId, nodes, prevObject) {
    const arr = (Array.isArray(nodes) ? nodes : []).map(enrichNode);
    const ids = () => arr.map((n) => n.nodeId);
    const runTraverse = (axis, sel) => {
      if (!arr.length) return makeWrapper(docId, [], w);
      const res = JSON.parse(__vegaDomTraverse(docId, JSON.stringify(ids()), axis, sel ? String(sel) : ""));
      return makeWrapper(docId, res, w);
    };

    const w = {
      length: arr.length,
      _nodes: arr,
      prevObject: prevObject || null,
      end: () => prevObject || makeWrapper(docId, []),
      [Symbol.iterator]: function*() {
        for (let i = 0; i < arr.length; i++) yield arr[i];
      },
      get: (idx) => (idx === undefined ? arr : arr[idx]),
      toArray: () => arr.map((n) => makeWrapper(docId, [n], w)),
      eq: (idx) => {
        const i = idx < 0 ? arr.length + idx : idx;
        return makeWrapper(docId, arr[i] ? [arr[i]] : [], w);
      },
      first: () => makeWrapper(docId, arr.length ? [arr[0]] : [], w),
      last: () => makeWrapper(docId, arr.length ? [arr[arr.length - 1]] : [], w),
      find: (sel) => {
        const found = [];
        for (const n of arr) {
          const res = JSON.parse(__vegaDomSelect(docId, n.nodeId, String(sel || "")));
          for (const r of res) found.push(r);
        }
        return makeWrapper(docId, found, w);
      },
      children: (sel) => runTraverse("children", sel),
      parent: () => runTraverse("parent", ""),
      parents: (sel) => runTraverse("parents", sel),
      closest: (sel) => runTraverse("closest", sel),
      next: (sel) => runTraverse("next", sel),
      nextAll: (sel) => runTraverse("nextAll", sel),
      nextUntil: (sel) => runTraverse("nextUntil", sel),
      prev: (sel) => runTraverse("prev", sel),
      prevAll: (sel) => runTraverse("prevAll", sel),
      siblings: (sel) => runTraverse("siblings", sel),
      is: (sel) => {
        if (!arr.length || !sel) return false;
        const matched = JSON.parse(__vegaDomFilter(docId, JSON.stringify(ids()), String(sel), false));
        return matched.length > 0;
      },
      filter: (selOrFn) => {
        if (typeof selOrFn === "function") {
          return makeWrapper(docId, arr.filter((n, i) => selOrFn.call(n, i, n)), w);
        }
        if (typeof selOrFn === "string") {
          return makeWrapper(docId, JSON.parse(__vegaDomFilter(docId, JSON.stringify(ids()), selOrFn, false)), w);
        }
        return w;
      },
      not: (selOrFn) => {
        if (typeof selOrFn === "function") {
          return makeWrapper(docId, arr.filter((n, i) => !selOrFn.call(n, i, n)), w);
        }
        if (typeof selOrFn === "string") {
          return makeWrapper(docId, JSON.parse(__vegaDomFilter(docId, JSON.stringify(ids()), selOrFn, true)), w);
        }
        return w;
      },
      clone: () => {
        if (!arr.length) return makeWrapper(docId, [], w);
        const cloned = JSON.parse(__vegaDomClone(docId, JSON.stringify(ids())));
        return makeWrapper(cloned.docId || docId, cloned.nodes || [], w);
      },
      remove: () => {
        if (arr.length) {
          __vegaDomRemove(docId, JSON.stringify(ids()));
        }
        return w;
      },
      text: () => (arr.length ? __vegaDomTexts(docId, JSON.stringify(ids())) : ""),
      html: () => (arr[0] ? __vegaDomHtml(docId, arr[0].nodeId) : ""),
      val: () => (arr[0] && arr[0].attrs ? arr[0].attrs.value : undefined),
      attr: (name, value) => {
        if (value !== undefined) {
          for (const n of arr) {
            if (!n.attrs) n.attrs = {};
            n.attrs[name] = String(value);
            n.attribs = n.attrs;
          }
          return w;
        }
        if (!arr[0] || !arr[0].attrs) return undefined;
        return arr[0].attrs[name];
      },
      data: (name) => {
        if (!arr[0] || !arr[0].attrs) return undefined;
        if (name === undefined) {
          const out = {};
          for (const [k, v] of Object.entries(arr[0].attrs)) {
            if (k.startsWith("data-")) out[k.slice(5)] = v;
          }
          return out;
        }
        return arr[0].attrs["data-" + name];
      },
      hasClass: (cls) => {
        if (!arr[0] || !arr[0].attrs || !arr[0].attrs.class) return false;
        return arr[0].attrs.class.split(/\s+/).includes(cls);
      },
      addClass: (cls) => {
        for (const n of arr) {
          const cur = (n.attrs && n.attrs.class) ? n.attrs.class.split(/\s+/).filter(Boolean) : [];
          if (!cur.includes(cls)) cur.push(cls);
          n.attrs.class = cur.join(" ");
        }
        return w;
      },
      removeClass: (cls) => {
        for (const n of arr) {
          const cur = (n.attrs && n.attrs.class) ? n.attrs.class.split(/\s+/).filter(c => c && c !== cls) : [];
          n.attrs.class = cur.join(" ");
        }
        return w;
      },
      each: (cb) => {
        for (let i = 0; i < arr.length; i++) {
          if (cb.call(arr[i], i, arr[i]) === false) break;
        }
        return w;
      },
      map: (cb) => {
        const mapped = [];
        for (let i = 0; i < arr.length; i++) {
          const r = cb.call(arr[i], i, arr[i]);
          if (r !== undefined && r !== null) mapped.push(r);
        }
        function wrapMap(list) {
          const nativeSlice = list.slice.bind(list);
          list.get = (i) => (i === undefined ? list : list[i]);
          list.toArray = () => list.slice();
          list.slice = (s, e) => wrapMap(nativeSlice(s, e));
          return list;
        }
        return wrapMap(mapped);
      },
      slice: (start, end) => makeWrapper(docId, arr.slice(start, end), w),
    };
    for (let i = 0; i < arr.length; i++) {
      w[i] = arr[i];
    }
    w.attribs = arr[0] ? arr[0].attrs || {} : {};
    return w;
  }

  globalThis.__vegaCheerioLoad = function(htmlStr) {
    const docId = __vegaDomParse(String(htmlStr || ""));
    const selectorFn = function(selector, context) {
      if (!selector) return makeWrapper(docId, []);
      if (typeof selector === "string") {
        const trimmed = selector.trim();
        if (trimmed.startsWith("<")) {
          const fragId = __vegaDomParse(trimmed);
          const fragNodes = JSON.parse(__vegaDomSelect(fragId, -1, "body > *"));
          return makeWrapper(fragId, fragNodes);
        }
        if (context && context._nodes && context._nodes.length) {
          return context.find(selector);
        }
        const nodes = JSON.parse(__vegaDomSelect(docId, -1, selector));
        return makeWrapper(docId, nodes);
      }
      if (selector._nodes) return selector;
      if (selector.nodeId !== undefined) return makeWrapper(selector.docId || docId, [selector]);
      return makeWrapper(docId, []);
    };
    selectorFn.html = () => __vegaDomRootHtml(docId);
    selectorFn.root = () => makeWrapper(docId, JSON.parse(__vegaDomSelect(docId, -1, "html")));
    return selectorFn;
  };
})();
"#;
