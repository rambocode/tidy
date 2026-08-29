// Configuration boundary for the Blog pipeline.
// Paths, route shapes, locales, and categories live here so the scripts hardcode
// nothing about any particular repository layout.
import fs from 'node:fs'
import path from 'node:path'

export const DEFAULT_CONFIG = {
  // Article sources, laid out as <contentDir>/<locale>/<permalink>.md
  contentDir: 'content/blog',
  // Build artifacts. These are data, not finished pages; the host framework renders them.
  outDir: '.blog-build',
  siteUrl: 'https://example.com',
  primaryLocale: 'zh',
  locales: ['zh'],
  // {locale} and {permalink} are the only placeholders
  articleRoute: '/{locale}/blog/{permalink}',
  indexRoute: '/{locale}/blog',
  rssRoute: '/{locale}/blog/rss.xml',
  // Stable category allowlist. Empty means unconstrained (not recommended:
  // categories grow ad hoc and stop meaning anything).
  categories: [],
  defaultAuthor: { name: null, url: null },
  // CJK by character, Latin by word. Applying one English constant to Chinese
  // under-reports reading time by roughly an order of magnitude.
  readingTime: { cjkCharsPerMinute: 350, wordsPerMinute: 220, minMinutes: 1 },
  // Controlled iframe allowlist, matched on exact host. Empty = iframes disabled.
  allowedIframeHosts: [],
  // RSS emits excerpts or full bodies
  rss: { mode: 'summary', maxItems: 20 },
  // Allowlist of end-of-article conversion blocks, referenced by frontmatter.cta
  ctaBlocks: ['default'],
  // Untranslated articles: 'hide' appear only in the locales that have them;
  // 'fallback' serves the primary-locale body.
  missingTranslation: 'hide',
  // Permalink change protection
  permalinkLockFile: 'content/blog/_permalinks.lock.json',
  redirectsFile: 'content/blog/_redirects.json',
}

export function loadConfig(cwd = process.cwd()) {
  const candidates = ['blog.config.json', 'docs/blog.config.json', '.blogrc.json']
  for (const rel of candidates) {
    const file = path.join(cwd, rel)
    if (fs.existsSync(file)) {
      const user = JSON.parse(fs.readFileSync(file, 'utf8'))
      return { ...DEFAULT_CONFIG, ...user, __file: rel, __cwd: cwd }
    }
  }
  return { ...DEFAULT_CONFIG, __file: null, __cwd: cwd }
}

export function routeFor(pattern, { locale, permalink }) {
  return pattern.replaceAll('{locale}', locale).replaceAll('{permalink}', permalink ?? '')
}

export function absoluteUrl(config, route) {
  return new URL(route, config.siteUrl).toString()
}
