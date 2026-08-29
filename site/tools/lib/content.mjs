// Content layer: read <contentDir>/<locale>/<permalink>.md into one structured set.
// Routes, RSS, sitemap, structured data, and locale switching all derive from this set,
// so no second article list can exist.
import fs from 'node:fs'
import path from 'node:path'
import matter from 'gray-matter'
import Ajv from 'ajv/dist/2020.js' // the schema targets draft 2020-12
import addFormats from 'ajv-formats'
import { fileURLToPath } from 'node:url'
import { renderArticle } from './markdown.mjs'
import { readingMinutes, countText } from './reading-time.mjs'
import { routeFor, absoluteUrl } from './config.mjs'

const HERE = path.dirname(fileURLToPath(import.meta.url))

export function loadSchema(cwd = process.cwd()) {
  const candidates = [
    path.join(cwd, 'docs/blog-frontmatter.schema.json'),
    path.join(HERE, '../../assets/blog-frontmatter.schema.json'),
    path.join(HERE, '../assets/blog-frontmatter.schema.json'),
  ]
  for (const file of candidates) {
    if (fs.existsSync(file)) return JSON.parse(fs.readFileSync(file, 'utf8'))
  }
  throw new Error('blog-frontmatter.schema.json not found; place it under docs/ or the skill assets/ directory')
}

function walk(dir) {
  if (!fs.existsSync(dir)) return []
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = path.join(dir, entry.name)
    if (entry.isDirectory()) return walk(full)
    if (/\.(md|markdown|html)$/i.test(entry.name) && !entry.name.startsWith('_')) return [full]
    return []
  })
}

export function collectPosts(config, { includeDrafts = false } = {}) {
  const cwd = config.__cwd || process.cwd()
  const contentDir = path.resolve(cwd, config.contentDir)
  const schema = loadSchema(cwd)
  const ajv = addFormats(new Ajv({ allErrors: true, useDefaults: true, strict: false }))
  const validate = ajv.compile(schema)

  const posts = []
  const issues = []

  for (const file of walk(contentDir)) {
    const rel = path.relative(cwd, file)
    const raw = fs.readFileSync(file, 'utf8')
    const { data, content } = matter(raw)

    // YAML parses an unquoted 2026-08-01 as a Date; normalize to a YYYY-MM-DD string
    // rather than forcing authors to quote every date.
    for (const key of ['publishedAt', 'updatedAt']) {
      if (data[key] instanceof Date) data[key] = data[key].toISOString().slice(0, 10)
    }

    if (!validate(data)) {
      for (const err of validate.errors || []) {
        issues.push({ file: rel, level: 'error', code: 'frontmatter', message: `${err.instancePath || '/'} ${err.message}` })
      }
      continue
    }

    // Directory locale must agree with frontmatter.lang, or locale switching points at the wrong file
    const dirLocale = path.relative(contentDir, file).split(path.sep)[0]
    if (dirLocale !== data.lang) {
      issues.push({ file: rel, level: 'error', code: 'locale-mismatch', message: `directory locale ${dirLocale} does not match frontmatter.lang ${data.lang}` })
    }
    if (!config.locales.includes(data.lang)) {
      issues.push({ file: rel, level: 'error', code: 'unknown-locale', message: `lang ${data.lang} is not among the locales declared in the specification` })
    }
    if (config.categories.length && !config.categories.includes(data.category)) {
      issues.push({ file: rel, level: 'error', code: 'unknown-category', message: `category "${data.category}" is not in the allowlist` })
    }
    if (data.updatedAt && data.updatedAt < data.publishedAt) {
      issues.push({ file: rel, level: 'error', code: 'date-order', message: 'updatedAt is earlier than publishedAt' })
    }
    if (data.cover && !data.coverAlt) {
      issues.push({ file: rel, level: 'error', code: 'missing-alt', message: 'cover is set but coverAlt is missing' })
    }
    if (data.cta && !config.ctaBlocks.includes(data.cta)) {
      issues.push({ file: rel, level: 'error', code: 'unknown-cta', message: `cta "${data.cta}" is not in ctaBlocks` })
    }

    const rendered = renderArticle(content, {
      allowedIframeHosts: config.allowedIframeHosts,
      siteUrl: config.siteUrl,
    })

    for (const item of rendered.unsafe) {
      issues.push({ file: rel, level: 'error', code: `unsafe-${item.kind}`, message: `rejected by the controlled-HTML boundary: ${item.detail}` })
    }
    for (const item of rendered.mediaIssues) {
      issues.push({ file: rel, level: 'error', code: item.kind, message: `figure image has no alt text: ${item.detail}` })
    }
    if (rendered.hasH1) {
      issues.push({ file: rel, level: 'error', code: 'body-h1', message: 'h1 found in the body; the page-level h1 comes from title, bodies start at h2' })
    }

    const counts = countText(rendered.plain)
    const minutes = data.readingMinutes || readingMinutes(rendered.plain, config.readingTime)
    const route = routeFor(config.articleRoute, { locale: data.lang, permalink: data.permalink })

    posts.push({
      file: rel,
      sourceFormat: /\.html$/i.test(file) ? 'html' : 'markdown',
      ...data,
      readingMinutes: minutes,
      wordCounts: counts,
      author: data.author || config.defaultAuthor.name,
      route,
      url: absoluteUrl(config, route),
      canonicalUrl: data.canonical || absoluteUrl(config, route),
      html: rendered.html,
      toc: data.toc === false ? [] : rendered.toc,
      links: rendered.links,
      images: rendered.images,
      excerpt: data.description,
    })
  }

  const visible = includeDrafts ? posts : posts.filter((p) => !p.draft)
  visible.sort((a, b) => (a.publishedAt < b.publishedAt ? 1 : a.publishedAt > b.publishedAt ? -1 : a.title.localeCompare(b.title)))
  return { posts: visible, all: posts, issues }
}

// Locale versions sharing one translationKey, used for hreflang and locale switching
export function groupTranslations(posts) {
  const groups = new Map()
  for (const post of posts) {
    if (!groups.has(post.translationKey)) groups.set(post.translationKey, [])
    groups.get(post.translationKey).push(post)
  }
  return groups
}

export function blogPostingJsonLd(post, config) {
  return {
    '@context': 'https://schema.org',
    '@type': 'BlogPosting',
    headline: post.title,
    description: post.description,
    inLanguage: post.lang,
    datePublished: post.publishedAt,
    dateModified: post.updatedAt || post.publishedAt,
    mainEntityOfPage: { '@type': 'WebPage', '@id': post.canonicalUrl },
    url: post.url,
    ...(post.author ? { author: { '@type': 'Person', name: post.author, ...(config.defaultAuthor.url ? { url: config.defaultAuthor.url } : {}) } } : {}),
    ...(post.cover ? { image: [absoluteUrl(config, post.cover)] } : {}),
    articleSection: post.category,
    ...(post.tags?.length ? { keywords: post.tags.join(', ') } : {}),
  }
}
