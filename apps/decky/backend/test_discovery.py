import copy
from pathlib import Path
import time
import unittest
from urllib.parse import parse_qs, urlsplit

from discovery import (MAX_HTML, RequestContext, _candidate, _normal, _variant, discover,
                       enrich_candidates, game_title, merge_candidates,
                       needs_fallback, parse_html, query_terms, safe_url,
                       validate_candidate)

FIXTURES = Path(__file__).with_name("fixtures") / "discovery"
CN = {"id": "kagura-cn", "role": "publisher", "adapter": "kagura_wordpress",
      "page_hosts": ["kagurafan.com"], "pages": ["https://kagurafan.com/"]}
EN = {"id": "kagura-en", "role": "publisher", "adapter": "kagura_woocommerce",
      "page_hosts": ["www.kaguragames.com"], "pages": ["https://www.kaguragames.com/patch/"]}
HG = {"id": "hgamer", "role": "fallback_indexer", "adapter": "hgamer_html",
      "page_hosts": ["hgamer.tw"], "pages": ["https://hgamer.tw/zh-CN/patches"]}
QUERY = {"appid": 1227890, "name": "Summer Memories", "name_hints": ["夏日狂想曲：乡间的难忘回忆"]}
CN_PAGE = "https://kagurafan.com/2020/06/14/base/"
EN_PAGE = "https://www.kaguragames.com/product/summer-memories-patch/"


def fixture(name):
    return (FIXTURES / (name + ".html")).read_text(encoding="utf-8")


class Fetcher:
    def __init__(self, routes):
        self.routes, self.calls = routes, []

    def __call__(self, url, rule):
        self.calls.append(url)
        result = self.routes[url] if url in self.routes else self.routes.get("*", "<title>unknown</title>")
        if isinstance(result, Exception):
            raise result
        if callable(result):
            result = result(url)
        return result if isinstance(result, tuple) else (200, result, {}, url)


def run(rule, query, routes, previous=None, budget=24):
    fetcher = Fetcher(routes)
    context = RequestContext(fetcher=fetcher, interval=0, budget=budget)
    return discover(rule, query, previous, context), fetcher


class DiscoveryTests(unittest.TestCase):
    def test_title_preserves_variant_and_numbers(self):
        self.assertEqual(game_title("Game+ Expansion DLC Patch"), "Game+ Expansion DLC")

    def test_dlc_and_demo_adjacent_to_chinese_are_not_base_variants(self):
        self.assertEqual(_variant("催眠學習 ―Secret Desire― DLC補丁"), "dlc")
        self.assertEqual(_variant("Game Demo补丁"), "demo")
        self.assertEqual(_variant("Demolition Game Patch"), "base")
        body = parse_html('<a href="https://store.steampowered.com/app/1788100/">Steam</a>')
        c = _candidate(HG, "催眠學習 ―Secret Desire― DLC補丁", "https://hgamer.tw/zh-TW/patches/saimin", body, {"appid": 1788100, "name": "Saimin Gakushū: Secret Desire"})[0]
        self.assertEqual(c["variant"], "dlc")
        self.assertEqual(c["parent_appid"], 1788100)
        self.assertIn("dlc_conditions_unknown", c["warnings"])
        self.assertNotEqual(_normal("Game+"), _normal("Game"))
        self.assertNotEqual(_normal("Game 2 Demo"), _normal("Game 2"))

    def test_query_terms_include_bundled_traditional_and_limit(self):
        q = {"name": "新搬来的大姐姐真是太可爱了", "name_hints": []}
        terms = query_terms(q)
        self.assertIn("新搬來的大姐姐真是太可愛了", terms)
        q["name_hints"] = ["English title", "another", "third"]
        self.assertEqual(len(query_terms(q)), 3)
        self.assertEqual(query_terms(q)[0], q["name"])
        self.assertIn("新搬來的大姐姐真是太可愛了", query_terms(q))

    def test_url_rejects_arbitrary_scheme_and_private_hosts(self):
        for url in ["http://hgamer.tw/a", "https://127.0.0.1/a", "https://localhost/a",
                    "https://x.local/a", "https://user:pass@hgamer.tw/a", "https://hgamer.tw:9/a",
                    "https://evil.example/a", "https://hgamer.tw/a\n"]:
            with self.subTest(url=url), self.assertRaises(ValueError):
                safe_url(url, ["hgamer.tw"])

    def test_cross_site_redirect_rejected(self):
        f = Fetcher({"*": (200, "html", {}, "https://evil.example/a")})
        with self.assertRaises(ValueError):
            RequestContext(f, interval=0).fetch("https://hgamer.tw/a", HG)

    def test_response_limit_and_deadline(self):
        for context in [RequestContext(Fetcher({"*": "x" * (MAX_HTML + 1)}), interval=0),
                        RequestContext(Fetcher({"*": "html"}), deadline=time.monotonic() - 1),
                        RequestContext(Fetcher({"*": "html"}), budget=0)]:
            with self.assertRaises(ValueError):
                context.fetch("https://hgamer.tw/a", HG)

    def test_duplicate_request_uses_shared_response(self):
        f = Fetcher({"*": "html"})
        ctx = RequestContext(f, interval=0)
        ctx.fetch("https://hgamer.tw/a", HG)
        ctx.fetch("https://hgamer.tw/a", HG)
        self.assertEqual(len(f.calls), 1)

    def test_retry_transient_bounded_and_retry_after_deferred(self):
        attempts = []
        def fetch(url, rule):
            attempts.append(url)
            return (503 if len(attempts) == 1 else 200, "html", {"Retry-After": "0"}, url)
        ctx = RequestContext(fetch, interval=0)
        self.assertEqual(ctx.fetch("https://hgamer.tw/a", HG)[0], 200)
        self.assertEqual(ctx.requests, 2)
        f = Fetcher({"*": (429, "", {"Retry-After": "100"}, "https://hgamer.tw/a")})
        with self.assertRaisesRegex(ValueError, "retry_deferred"):
            RequestContext(f, interval=0).fetch("https://hgamer.tw/a", HG)
        self.assertEqual(len(f.calls), 1)

    def test_en_real_cards_exclude_sidebar(self):
        html = fixture("en_index") + '<aside class="widget"><div class="product type-product"><p class="name">Summer Memories Patch</p><a href="https://www.kaguragames.com/product/evil/">bad</a></div></aside>'
        result, f = run(EN, QUERY, {EN["pages"][0]: html, EN_PAGE: fixture("en_product"), "*": ValueError("unavailable")})
        self.assertEqual(result["status"], "partial")  # expansion detail absent
        base = next(c for c in result["candidates"] if c["variant"] == "base")
        self.assertEqual(base["version_raw"], "3")
        self.assertEqual(base["languages"], ["en"])
        self.assertEqual(len(base["mirrors"]), 3)
        self.assertEqual(base["format_observed"], "exe")
        self.assertEqual(base["match_basis"], "title")
        self.assertIsNone(base["game_appid"])
        self.assertFalse(any("/evil/" in u for u in f.calls))

    def test_runtime_new_game_without_static_table(self):
        html = '<div class="product type-product"><p class="name">New Robot 72 Patch</p><a href="/product/new-robot/">New Robot 72 Patch</a></div>'
        detail = '<h1>New Robot 72 Patch</h1><div id="tab-patch"><a href="https://store.steampowered.com/app/987654/">Steam</a>v1.5 <a href="https://files.example/new-robot-v1.5.exe">Mirror 1</a></div>'
        result, _ = run(EN, {"appid": 987654, "name": "New Robot 72"}, {EN["pages"][0]: html, "https://www.kaguragames.com/product/new-robot/": detail})
        self.assertEqual(result["status"], "complete")
        self.assertEqual(result["candidates"][0]["match_basis"], "appid")

    def test_wrong_appid_same_title_rejected(self):
        body = parse_html('<a href="https://store.steampowered.com/app/999/">Steam</a>')
        self.assertEqual(_candidate(EN, "Summer Memories Patch", EN_PAGE, body, QUERY), [])

    def test_cn_scope_and_known_explicit_condition(self):
        html = fixture("cn_base") + '<aside><a href="https://files.example/evil.zip">下载</a></aside>'
        query = dict(QUERY, upstream_pages=[CN_PAGE])
        result, _ = run(CN, query, {CN_PAGE: html})
        candidate = result["candidates"][0]
        self.assertEqual(candidate["excludes_dlc"], [1390150])
        self.assertEqual(candidate["version_raw"], "1.03")
        self.assertEqual(len(candidate["mirrors"]), 5)
        self.assertFalse(any("evil" in m["url"] for m in candidate["mirrors"]))
        self.assertNotIn("103", candidate["version_evidence"])

    def test_cn_explicit_dlc_purchase_condition_and_version_password(self):
        body = parse_html('<p>必须有购买 DLC 才能使用！中文补丁v2.02.1，旧说明v2.021。百度提取码：v202</p>')
        c = _candidate(CN, "夏日狂想曲+", CN_PAGE, body, dict(QUERY, name_hints=["夏日狂想曲+"]))[0]
        self.assertEqual(c["requires_dlc"], [1390150])
        self.assertEqual(c["version_evidence"], ["2.02.1", "2.021"])
        self.assertIn("version_conflict", c["warnings"])

    def test_dlc_conditions_not_generalized(self):
        body = parse_html("<p>没有购买DLC才能使用v1.0</p>")
        c = _candidate(CN, "Different Game", CN_PAGE, body, {"appid": 777, "name": "Different Game"})[0]
        self.assertIsNone(c["excludes_dlc"])

    def test_same_page_versions_and_artifacts_do_not_collapse(self):
        body = parse_html('<p>Steam v1.0 v2.0</p>')
        mirrors = [{"label": "Mirror", "url": "https://files.example/Game-EN-Patch-v" + version + suffix + ".exe?sig=123", "format": "exe"}
                   for version, suffix in [("1.0", ""), ("2.0", ""), ("2.0", "-other")]]
        candidates = _candidate(EN, "Summer Memories Patch", EN_PAGE, body, QUERY, mirrors=mirrors)
        self.assertEqual(len(candidates), 3)
        self.assertEqual(len({c["candidate_id"] for c in candidates}), 3)
        changed = copy.deepcopy(mirrors)
        changed.reverse()
        for m in changed:
            m["url"] = m["url"].replace("sig=123", "sig=999")
        self.assertEqual({c["candidate_id"] for c in candidates}, {c["candidate_id"] for c in _candidate(EN, "Summer Memories Patch", EN_PAGE, body, QUERY, mirrors=changed)})

    def test_share_token_is_not_a_version(self):
        body = parse_html("<p>Steam 中文无码补丁包v1.03</p>")
        mirrors = [{"label": "下载", "url": "https://share.example/v202/folder", "format": "web"},
                   {"label": "Mirror 1", "url": "https://files.example/Summer-Memories-CN-Patch-v1.03.exe", "format": "exe"}]
        candidates = _candidate(CN, "夏日狂想曲：乡间的难忘回忆", CN_PAGE, body, QUERY, mirrors=mirrors)
        self.assertEqual(len(candidates), 1)
        self.assertEqual(candidates[0]["version_raw"], "1.03")
        self.assertEqual(len(candidates[0]["mirrors"]), 2)

    def test_cn_old_game_category_and_pagination(self):
        category = "https://kagurafan.com/category/old/"
        next_page = category + "page/2/"
        homepage = '<section class="widget_categories"><a href="' + category + '">夏日狂想曲：乡间的难忘回忆</a></section><a class="next" href="/page/2/">next</a>'
        page1 = '<article><h2 class="entry-title"><a href="/other/">Unrelated</a></h2></article><a class="next" href="' + next_page + '">next</a>'
        page2 = '<article><h2 class="entry-title"><a href="' + CN_PAGE + '">夏日狂想曲：乡间的难忘回忆</a></h2></article>'
        result, f = run(CN, QUERY, {CN["pages"][0]: homepage, category: page1, next_page: page2, CN_PAGE: fixture("cn_base")})
        self.assertEqual(result["status"], "complete", result)
        self.assertEqual(len(result["candidates"]), 1)
        self.assertNotIn("https://kagurafan.com/page/2/", f.calls)
        previous = {"index": result["index"]}
        repeated, rf = run(CN, QUERY, {category: page1, next_page: page2, CN_PAGE: fixture("cn_base")}, previous)
        self.assertEqual(repeated["status"], "complete")
        self.assertNotIn("https://kagurafan.com/page/2/", rf.calls)

    def test_cn_category_structure_failure_not_negative(self):
        category = "https://kagurafan.com/category/old/"
        homepage = '<section class="widget_categories"><a href="' + category + '">夏日狂想曲：乡间的难忘回忆</a></section>'
        result, _ = run(CN, QUERY, {CN["pages"][0]: homepage, category: "<title>unexpected</title>"})
        self.assertEqual(result["status"], "failed")

    def test_large_realistic_index_under_four_mib(self):
        html = "<!--" + "x" * (2300 * 1024) + "-->" + fixture("en_index")
        result, _ = run(EN, QUERY, {EN["pages"][0]: html, EN_PAGE: fixture("en_product"), "*": ValueError("missing expansion")})
        self.assertTrue(result["candidates"])
        self.assertNotIn("discovery_html_too_large", [d["code"] for d in result["diagnostics"]])

    def test_hg_zero_recommendations_not_results(self):
        result, f = run(HG, {"appid": 123, "name": "codex-feasibility-no-game-01a106ff"}, {"*": fixture("hg_empty_search")})
        self.assertEqual(result["status"], "complete")
        self.assertEqual(result["candidates"], [])
        self.assertEqual(len(f.calls), 1)

    def test_hg_structure_change_not_zero(self):
        result, _ = run(HG, {"appid": 1, "name": "game"}, {"*": "<h1>Search</h1><h2>Recommendations</h2>"})
        self.assertEqual(result["status"], "failed")

    def test_hg_positive_count_with_changed_group_not_negative(self):
        html = '<section><h1>搜尋</h1>「game」找到1筆結果</section><section><div><h3>遊戲</h3></div><a href="/zh-CN/games/game">game</a></section>'
        result, _ = run(HG, {"appid": 1, "name": "game"}, {"*": html})
        self.assertEqual(result["status"], "failed")
        self.assertEqual(result["diagnostics"][0]["code"], "discovery_search_structure_changed")

    def test_hg_articles_only_complete_empty(self):
        html = '<section><h1>搜尋</h1>「game」找到1筆結果</section><section><div><h2>文章</h2></div><a href="/zh-CN/posts/game">game</a></section>'
        result, f = run(HG, {"appid": 1, "name": "game"}, {"*": html})
        self.assertEqual(result["status"], "complete")
        self.assertEqual(result["candidates"], [])
        self.assertEqual(len(f.calls), 1)

    def test_hg_simplified_zero_traditional_positive_unknown_publisher(self):
        def routes(url):
            p = urlsplit(url)
            if p.path.endswith("/search"):
                q = parse_qs(p.query)["q"][0]
                name = "hg_other_traditional" if "來" in q else "hg_other_search"
                return 200, fixture(name), {}, url.replace("zh-CN", "zh-TW")
            if "/games/" in p.path:
                return fixture("hg_other")
            return fixture("hg_other_patch")
        result, f = run(HG, {"appid": 4483210, "name": "新搬来的大姐姐真是太可爱了"}, {"*": routes})
        self.assertEqual(result["status"], "complete", result)
        self.assertEqual(len(result["candidates"]), 1)
        c = result["candidates"][0]
        self.assertEqual(c["game_appid"], 4483210)
        self.assertEqual(c["upstream_page"], "https://azucat.net/patch/")
        self.assertEqual(c["format_reported"], "zip")
        self.assertEqual(c["capability"], "discover_only")
        self.assertTrue(any("Summer" not in a for a in result["identities"][0]["aliases"]))
        self.assertFalse(any("sini" in u.lower() or "/posts/" in u for u in f.calls))

    def test_hg_recommended_appid_not_identity(self):
        html = '<div data-patch-body="true"><div><h1>Other Game Patch</h1></div><section><div><h2>相關文章</h2></div><a href="https://store.steampowered.com/app/1227890/">recommendation</a></section></div>'
        search = '<section><h1>搜尋</h1>「Summer Memories」找到 1 筆結果</section><section><div><h2>補丁</h2></div><a href="/zh-CN/patches/other">Other</a></section>'
        result, _ = run(HG, {"appid": 1227890, "name": "Summer Memories"}, {"*": search, "https://hgamer.tw/zh-CN/patches/other": html})
        self.assertEqual(result["candidates"], [])

    def test_identity_enrichment_requires_full_verified_alias(self):
        c = _candidate(EN, "Summer Memories Patch", EN_PAGE, None, QUERY)[0]
        enriched = enrich_candidates([c], [{"appid": 1227890, "page": "https://hgamer.tw/zh-CN/games/a", "aliases": ["Summer Memories"]}])[0]
        self.assertEqual(enriched["match_basis"], "verified_alias")
        self.assertIsNone(enrich_candidates([c], [{"appid": 9, "page": "https://hgamer.tw/a", "aliases": ["Summer Memories+"]}])[0]["game_appid"])

    def test_format_conflict_is_discovery_only(self):
        body = parse_html('<a href="https://store.steampowered.com/app/1227890/">Steam</a>')
        c = _candidate(HG, "Summer Memories Patch", "https://hgamer.tw/zh-CN/patches/a", body, QUERY, reported="zip",
                       mirrors=[{"url": "https://files.example/game.exe", "label": "download", "format": "exe"}])[0]
        self.assertIn("artifact_format_conflict", c["warnings"])
        self.assertEqual(c["capability"], "discover_only")

    def test_index_continuation_budget_and_cycle(self):
        index = '<div class="product type-product"><p class="name">Other Patch</p><a href="/product/other/">Other</a></div><a class="next" href="/patch/">next</a>'
        result, _ = run(EN, QUERY, {"*": index})
        self.assertEqual(result["status"], "partial")
        self.assertIn(EN["pages"][0], result["index"]["pending"])
        self.assertGreater(result["index"]["checked_at"], 0)

    def test_known_index_skips_index_fetch_and_preserves_timestamp(self):
        previous = {"index": {"records": [{"title": "Summer Memories Patch", "page": EN_PAGE, "kind": "product"}], "pending": [], "checked_at": 123}}
        result, f = run(EN, QUERY, {EN_PAGE: fixture("en_product")}, previous)
        self.assertEqual(f.calls, [EN_PAGE])
        self.assertEqual(result["index"]["checked_at"], 123)

    def test_force_recrawls_new_record_despite_hot_index(self):
        previous = {"index": {"records": [{"title": "Other Patch", "page": EN_PAGE, "kind": "product"}], "pending": [], "checked_at": 123}}
        html = '<div class="product type-product"><p class="name">A Game Patch</p><a href="/product/a-game/">A Game Patch</a></div>'
        detail = '<h1>A Game Patch</h1><div id="tab-patch">v1</div>'
        result, f = run(EN, {"appid": 123, "name": "A Game", "force": True}, {EN["pages"][0]: html, "https://www.kaguragames.com/product/a-game/": detail}, previous)
        self.assertEqual(result["status"], "complete")
        self.assertEqual(len(result["candidates"]), 1)
        self.assertIn(EN["pages"][0], f.calls)

    def test_cn_unknown_game_reserves_fallback_budget(self):
        page = '<article><h2 class="entry-title"><a href="/other/">Other</a></h2></article><a class="next" href="/page/2/">next</a>'
        page2 = page.replace("/page/2/", "/page/3/")
        result, f = run(CN, {"appid": 4483210, "name": "Unknown Game"}, {CN["pages"][0]: page, "https://kagurafan.com/page/2/": page2})
        self.assertEqual(result["status"], "partial")
        self.assertEqual(len(f.calls), 2)
        self.assertEqual(result["index"]["pending"], ["https://kagurafan.com/page/3/"])

    def test_detail_failure_retains_page_level_candidate(self):
        result, _ = run(EN, QUERY, {EN["pages"][0]: fixture("en_index"), "*": ValueError("403")})
        self.assertEqual(result["status"], "partial")
        self.assertIn("detail_unavailable", result["candidates"][0]["warnings"])

    def test_fallback_clear_exe_candidate_and_language(self):
        body = parse_html('<a href="https://store.steampowered.com/app/1227890/">Steam</a>English v1')
        c = _candidate(EN, "Summer Memories Patch", EN_PAGE, body, QUERY)[0]
        self.assertFalse(needs_fallback([c], QUERY))
        self.assertTrue(needs_fallback([c], dict(QUERY, preferred_language="zh-CN")))
        self.assertTrue(needs_fallback([dict(c, cache_status="stale")], QUERY))

    def test_candidate_validation_does_not_grant_installability(self):
        c = _candidate(EN, "Summer Memories Patch", EN_PAGE, None, QUERY)[0]
        self.assertEqual(len(merge_candidates([c, c])), 1)
        for changed in [dict(c, capability="publisher_zip"), dict(c, match_basis="anything"), dict(c, source_page="https://127.0.0.1/a")]:
            with self.assertRaises(ValueError):
                validate_candidate(changed, EN)


if __name__ == "__main__":
    unittest.main()
