#!/usr/bin/env node
// Content gate. Wire into the repository check script and CI; a non-zero exit blocks publication.
// Covers the frontmatter contract, cross-locale consistency, the controlled-HTML boundary,
// dead links, missing images, and permalink change protection.
import fs from 'node:fs'
import path from 'node:path'
import { loadConfig, routeFor } from './lib/config.mjs'
import { collectPosts, groupTranslations } from './lib/content.mjs'

const config = loadConfig()
const cwd = config.__cwd
const strict = process.argv.includes('--strict') // treat warnings as failures too

const { posts, all, issues } = collectPosts(config, { includeDrafts: true })
const published = all.filter((p) => !p.draft)

// 1. Permalinks must be unique within one locale
const seen = new Map()
for (const post of all) {
  const key = `${post.lang}:${post.permalink}`
  if (seen.has(key)) {
    issues.push({ file: post.file, level: 'error', code: 'duplicate-permalink', message: `permalink collides with ${seen.get(key)}` })
  } else seen.set(key, post.file)
}

// 2. Permalinks must agree inside a translationKey group, or locale switching lands on the wrong URL
const groups = groupTranslations(all)
for (const [key, items] of groups) {
  const permalinks = new Set(items.map((p) => p.permalink))
  if (permalinks.size > 1) {
    issues.push({ file: items.map((p) => p.file).join(', '), level: 'error', code: 'translation-permalink-drift', message: `translationKey ${key} has multiple permalinks: ${[...permalinks].join(', ')}` })
  }
  const langs = new Set(items.map((p) => p.lang))
  if (!langs.has(config.primaryLocale) && items.some((p) => !p.draft)) {
    issues.push({ file: items[0].file, level: 'warn', code: 'missing-primary-locale', message: `translationKey ${key} has no ${config.primaryLocale} (primary locale) version` })
  }
  const missing = config.locales.filter((l) => !langs.has(l))
  if (missing.length && config.missingTranslation === 'hide' && items.some((p) => !p.draft)) {
    issues.push({ file: items[0].file, level: 'info', code: 'partial-translation', message: `untranslated locales: ${missing.join(', ')} (the hide strategy emits no route or hreflang for them)` })
  }
}

// 3. Internal links must resolve to routes that exist
const norm = (r) => r.replace(/\/$/, '') || '/'
const knownRoutes = new Set([
  ...published.map((p) => norm(p.route)),
  ...config.locales.map((l) => norm(routeFor(config.indexRoute, { locale: l }))),
])
const blogPrefixes = config.locales.map((l) => norm(routeFor(config.indexRoute, { locale: l })))
for (const post of published) {
  for (const href of post.links) {
    if (!href.startsWith('/')) continue
    const clean = href.split('#')[0].split('?')[0].replace(/\/$/, '') || '/'
    // Only links inside the blog namespace are checked here; other routes belong to the host build
    if (!blogPrefixes.some((prefix) => clean.startsWith(prefix))) continue
    if (!knownRoutes.has(clean)) {
      issues.push({ file: post.file, level: 'error', code: 'dead-link', message: `internal link points at a non-existent article: ${href}` })
    }
  }
  for (const src of post.images) {
    if (/^(https?:)?\/\//.test(src) || src.startsWith('data:')) continue
    const candidates = [
      path.resolve(cwd, 'public', src.replace(/^\//, '')),
      path.resolve(cwd, path.dirname(post.file), src),
    ]
    if (!candidates.some((f) => fs.existsSync(f))) {
      issues.push({ file: post.file, level: 'error', code: 'missing-image', message: `image not found: ${src}` })
    }
  }
}

// 4. Permalink change protection: a published URL cannot be renamed silently
const lockFile = path.resolve(cwd, config.permalinkLockFile)
const redirectsFile = path.resolve(cwd, config.redirectsFile)
const lock = fs.existsSync(lockFile) ? JSON.parse(fs.readFileSync(lockFile, 'utf8')) : null
const redirects = fs.existsSync(redirectsFile) ? JSON.parse(fs.readFileSync(redirectsFile, 'utf8')) : {}
const currentRoutes = published.map((p) => p.route).sort()
if (lock) {
  for (const oldRoute of lock.routes || []) {
    if (!currentRoutes.includes(oldRoute) && !redirects[oldRoute]) {
      issues.push({ file: config.permalinkLockFile, level: 'error', code: 'permalink-removed', message: `published route ${oldRoute} disappeared with no redirect; add a 301 in ${config.redirectsFile} first` })
    }
  }
}

// 5. Summary
const order = { error: 0, warn: 1, info: 2 }
issues.sort((a, b) => order[a.level] - order[b.level])
const errors = issues.filter((i) => i.level === 'error')
const warns = issues.filter((i) => i.level === 'warn')

const icon = { error: '✗', warn: '!', info: '·' }
for (const issue of issues) {
  console.log(`${icon[issue.level]} [${issue.code}] ${issue.file}\n    ${issue.message}`)
}

console.log('')
console.log(`${all.length} articles (${published.length} published / ${all.length - published.length} draft)`)
console.log(`locales ${config.locales.join(', ')}, ${groups.size} translation groups`)
console.log(`${errors.length} errors, ${warns.length} warnings`)

if (process.argv.includes('--write-lock')) {
  fs.mkdirSync(path.dirname(lockFile), { recursive: true })
  fs.writeFileSync(lockFile, `${JSON.stringify({ routes: currentRoutes }, null, 2)}\n`, 'utf8')
  console.log(`✓ updated ${config.permalinkLockFile}`)
}

if (errors.length || (strict && warns.length)) process.exit(1)
