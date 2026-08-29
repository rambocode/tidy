#!/usr/bin/env node
// Create an article skeleton. Publishing should require this one command and nothing
// else — no route file, no component, no hand-maintained list.
//
//   node scripts/new-post.mjs --title "Find the app eating your bandwidth" --permalink find-network-hog-app --category "Performance"
//   node scripts/new-post.mjs --translate find-network-hog-app --locale en
//
import fs from 'node:fs'
import path from 'node:path'
import { loadConfig } from './lib/config.mjs'
import { collectPosts } from './lib/content.mjs'

function parseArgs(argv) {
  const out = {}
  for (let i = 0; i < argv.length; i += 1) {
    const token = argv[i]
    if (!token.startsWith('--')) continue
    const key = token.slice(2)
    const next = argv[i + 1]
    if (!next || next.startsWith('--')) out[key] = true
    else { out[key] = next; i += 1 }
  }
  return out
}

const args = parseArgs(process.argv.slice(2))
const config = loadConfig()
const cwd = config.__cwd

const locale = args.locale || config.primaryLocale
let permalink = args.permalink || args.translate
let translationKey = args.translationKey || permalink
let title = args.title
let category = args.category

// --translate inherits permalink / translationKey / category from an existing locale
// version, which is what keeps a translation on the same URL.
if (args.translate) {
  const { all } = collectPosts(config, { includeDrafts: true })
  const origin = all.find((p) => p.permalink === args.translate || p.translationKey === args.translate)
  if (!origin) {
    console.error(`✗ no article found to translate: ${args.translate}`)
    process.exit(1)
  }
  permalink = origin.permalink
  translationKey = origin.translationKey
  category = category || origin.category
  title = title || `TODO: translation of "${origin.title}"`
}

if (!permalink || !title) {
  console.error('usage: node scripts/new-post.mjs --title "Title" --permalink stable-slug --category "Category" [--locale zh]')
  console.error('       node scripts/new-post.mjs --translate stable-slug --locale en')
  process.exit(1)
}
if (!/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(permalink)) {
  console.error(`✗ permalink must be lowercase-with-hyphens: ${permalink}`)
  console.error('  For a CJK title, write an English slug by hand; do not transliterate.')
  process.exit(1)
}
if (!config.locales.includes(locale)) {
  console.error(`✗ locale ${locale} is not one of ${config.locales.join(', ')}`)
  process.exit(1)
}

const target = path.resolve(cwd, config.contentDir, locale, `${permalink}.md`)
if (fs.existsSync(target)) {
  console.error(`✗ already exists: ${path.relative(cwd, target)}`)
  process.exit(1)
}

const today = new Date().toISOString().slice(0, 10)
const body = `---
title: ${JSON.stringify(title)}
description: "TODO: one or two sentences of reader value, 20-160 characters; used verbatim as the meta description and RSS excerpt"
lang: ${locale}
permalink: ${permalink}
translationKey: ${translationKey}
category: ${JSON.stringify(category || 'TODO')}
tags: []
publishedAt: ${today}
updatedAt: null
readingMinutes: null
author: null
cover: null
coverAlt: null
featured: false
draft: true
noindex: false
canonical: null
toc: true
cta: null
series: null
seriesOrder: null
related: []
---

<!-- Bodies start at h2. The page-level h1 comes from title; an h1 here is rejected. -->

## Who this is for, and where they are stuck

TODO: name the reader and their concrete situation. Not a company introduction.

## The answer, standing on its own

TODO: this section should be useful even to a reader who never adopts the product.

## Where the product actually shortens a step

TODO: introduce the product only where it removes a real step, and prove it with
reproducible UI or released behavior.

> [!NOTE]
> Callouts support NOTE / TIP / WARNING / IMPORTANT / CAUTION.

## Next step

TODO: one specific next action.
`

fs.mkdirSync(path.dirname(target), { recursive: true })
fs.writeFileSync(target, body, 'utf8')
console.log(`✓ created ${path.relative(cwd, target)}`)
console.log('  When the body is written, set draft to false and run node scripts/validate-content.mjs')
