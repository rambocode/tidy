#!/usr/bin/env node
// Compile Markdown and controlled HTML into rendering artifacts.
// The output is data, not finished pages: the host framework (Astro / Next / Nuxt / a static
// generator) reads this JSON and renders routes, so the pipeline binds to no framework and
// does not fight the repository's existing component system.
import fs from 'node:fs'
import path from 'node:path'
import { loadConfig, routeFor, absoluteUrl } from './lib/config.mjs'
import { collectPosts, groupTranslations, blogPostingJsonLd } from './lib/content.mjs'

const config = loadConfig()
const cwd = config.__cwd
const outDir = path.resolve(cwd, config.outDir)

const { posts, issues } = collectPosts(config, { includeDrafts: false })
const blocking = issues.filter((i) => i.level === 'error')
if (blocking.length) {
  for (const issue of blocking) console.error(`✗ [${issue.code}] ${issue.file}: ${issue.message}`)
  console.error('\nBuild aborted. Fix the errors above, or run node scripts/validate-content.mjs for the full report.')
  process.exit(1)
}

const groups = groupTranslations(posts)

const xml = (s) => String(s ?? '').replace(/[<>&'"]/g, (c) => ({ '<': '&lt;', '>': '&gt;', '&': '&amp;', "'": '&apos;', '"': '&quot;' }[c]))
const rfc822 = (date) => new Date(`${date}T00:00:00Z`).toUTCString()

function alternatesFor(post) {
  return (groups.get(post.translationKey) || [])
    .map((p) => ({ lang: p.lang, url: p.url, route: p.route, title: p.title }))
    .sort((a, b) => a.lang.localeCompare(b.lang))
}

function relatedFor(post) {
  if (post.related?.length) {
    return post.related
      .map((key) => (groups.get(key) || []).find((p) => p.lang === post.lang))
      .filter(Boolean)
      .map((p) => ({ title: p.title, route: p.route, description: p.description }))
  }
  return posts
    .filter((p) => p.lang === post.lang && p.translationKey !== post.translationKey)
    .map((p) => ({
      post: p,
      score: (p.category === post.category ? 2 : 0) + (p.tags || []).filter((t) => (post.tags || []).includes(t)).length,
    }))
    .filter((x) => x.score > 0)
    .sort((a, b) => b.score - a.score || (a.post.publishedAt < b.post.publishedAt ? 1 : -1))
    .slice(0, 3)
    .map((x) => ({ title: x.post.title, route: x.post.route, description: x.post.description }))
}

function seriesNav(post) {
  if (!post.series) return null
  const siblings = posts
    .filter((p) => p.lang === post.lang && p.series === post.series)
    .sort((a, b) => (a.seriesOrder ?? 0) - (b.seriesOrder ?? 0))
  const index = siblings.findIndex((p) => p.translationKey === post.translationKey)
  return {
    name: post.series,
    prev: index > 0 ? { title: siblings[index - 1].title, route: siblings[index - 1].route } : null,
    next: index >= 0 && index < siblings.length - 1 ? { title: siblings[index + 1].title, route: siblings[index + 1].route } : null,
  }
}

fs.rmSync(outDir, { recursive: true, force: true })
fs.mkdirSync(outDir, { recursive: true })

// 1. One rendering artifact per article
for (const post of posts) {
  const payload = {
    title: post.title,
    description: post.description,
    lang: post.lang,
    permalink: post.permalink,
    translationKey: post.translationKey,
    category: post.category,
    tags: post.tags || [],
    publishedAt: post.publishedAt,
    updatedAt: post.updatedAt,
    readingMinutes: post.readingMinutes,
    wordCounts: post.wordCounts,
    author: post.author,
    cover: post.cover,
    coverAlt: post.coverAlt,
    featured: post.featured,
    noindex: post.noindex,
    route: post.route,
    url: post.url,
    canonicalUrl: post.canonicalUrl,
    sourceFile: post.file,
    sourceFormat: post.sourceFormat,
    cta: post.cta || config.ctaBlocks[0] || null,
    toc: post.toc,
    html: post.html,
    alternates: alternatesFor(post),
    related: relatedFor(post),
    series: seriesNav(post),
    jsonLd: blogPostingJsonLd(post, config),
  }
  const file = path.join(outDir, 'posts', post.lang, `${post.permalink}.json`)
  fs.mkdirSync(path.dirname(file), { recursive: true })
  fs.writeFileSync(file, `${JSON.stringify(payload, null, 2)}\n`, 'utf8')
}

// 2. Index data: grouped by locale, then by category (a category-column blog index consumes this directly)
const index = {}
for (const locale of config.locales) {
  const localePosts = posts.filter((p) => p.lang === locale)
  if (!localePosts.length) continue
  const byCategory = []
  for (const category of config.categories.length ? config.categories : [...new Set(localePosts.map((p) => p.category))]) {
    const items = localePosts.filter((p) => p.category === category)
    if (items.length) {
      byCategory.push({
        category,
        items: items.map((p) => ({
          title: p.title, description: p.description, route: p.route,
          publishedAt: p.publishedAt, updatedAt: p.updatedAt,
          readingMinutes: p.readingMinutes, author: p.author,
          cover: p.cover, coverAlt: p.coverAlt, featured: p.featured, tags: p.tags || [],
        })),
      })
    }
  }
  index[locale] = {
    route: routeFor(config.indexRoute, { locale }),
    url: absoluteUrl(config, routeFor(config.indexRoute, { locale })),
    rss: routeFor(config.rssRoute, { locale }),
    total: localePosts.length,
    featured: localePosts.filter((p) => p.featured).map((p) => p.route),
    categories: byCategory,
  }
}
fs.writeFileSync(path.join(outDir, 'index.json'), `${JSON.stringify(index, null, 2)}\n`, 'utf8')

// 3. One RSS feed per locale
fs.mkdirSync(path.join(outDir, 'rss'), { recursive: true })
for (const locale of Object.keys(index)) {
  const items = posts.filter((p) => p.lang === locale && !p.noindex).slice(0, config.rss.maxItems)
  const body = items.map((p) => `    <item>
      <title>${xml(p.title)}</title>
      <link>${xml(p.url)}</link>
      <guid isPermaLink="true">${xml(p.url)}</guid>
      <pubDate>${rfc822(p.publishedAt)}</pubDate>
      <category>${xml(p.category)}</category>
      ${p.author ? `<dc:creator>${xml(p.author)}</dc:creator>` : ''}
      <description><![CDATA[${config.rss.mode === 'full' ? p.html : p.description}]]></description>
    </item>`).join('\n')
  const feed = `<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:atom="http://www.w3.org/2005/Atom" xmlns:dc="http://purl.org/dc/elements/1.1/">
  <channel>
    <title>${xml(index[locale].route)}</title>
    <link>${xml(index[locale].url)}</link>
    <description>${xml(`Blog (${locale})`)}</description>
    <language>${xml(locale)}</language>
    <atom:link href="${xml(absoluteUrl(config, index[locale].rss))}" rel="self" type="application/rss+xml"/>
${body}
  </channel>
</rss>
`
  fs.writeFileSync(path.join(outDir, 'rss', `${locale}.xml`), feed, 'utf8')
}

// 4. Sitemap fragment with hreflang alternates; drafts and noindex articles are excluded
const urls = posts.filter((p) => !p.noindex).map((post) => {
  const alts = alternatesFor(post)
  const links = alts.map((a) => `    <xhtml:link rel="alternate" hreflang="${xml(a.lang)}" href="${xml(a.url)}"/>`).join('\n')
  const xDefault = alts.find((a) => a.lang === config.primaryLocale)
  return `  <url>
    <loc>${xml(post.url)}</loc>
    <lastmod>${xml(post.updatedAt || post.publishedAt)}</lastmod>
${links}
${xDefault ? `    <xhtml:link rel="alternate" hreflang="x-default" href="${xml(xDefault.url)}"/>` : ''}
  </url>`
}).join('\n')
fs.writeFileSync(path.join(outDir, 'sitemap-blog.xml'), `<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:xhtml="http://www.w3.org/1999/xhtml">
${urls}
</urlset>
`, 'utf8')

// 5. Redirect table passed through verbatim for the host framework or CDN
const redirectsFile = path.resolve(cwd, config.redirectsFile)
fs.writeFileSync(
  path.join(outDir, 'redirects.json'),
  `${JSON.stringify(fs.existsSync(redirectsFile) ? JSON.parse(fs.readFileSync(redirectsFile, 'utf8')) : {}, null, 2)}\n`,
  'utf8',
)

console.log(`✓ built ${posts.length} articles → ${config.outDir}`)
for (const locale of Object.keys(index)) {
  console.log(`  ${locale}: ${index[locale].total} articles, ${index[locale].categories.length} categories`)
}
console.log('  artifacts: posts/, index.json, rss/, sitemap-blog.xml, redirects.json')
