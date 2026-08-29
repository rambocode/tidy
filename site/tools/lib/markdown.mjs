// The single rendering entry point for Markdown and controlled HTML → sanitized HTML.
// No page component may assemble its own markdown pipeline; a second pipeline
// means a second security boundary.
import { unified } from 'unified'
import remarkParse from 'remark-parse'
import remarkGfm from 'remark-gfm'
import remarkRehype from 'remark-rehype'
import rehypeRaw from 'rehype-raw'
import rehypeSanitize, { defaultSchema } from 'rehype-sanitize'
import rehypeSlug from 'rehype-slug'
import rehypeStringify from 'rehype-stringify'
import { visit } from 'unist-util-visit'
import { toString as hastToString } from 'hast-util-to-string'

const EVENT_ATTR = /^on/i
const DANGEROUS_URL = /^\s*(javascript|vbscript|data:text\/html)/i

export function buildSchema({ allowedIframeHosts = [] } = {}) {
  const schema = structuredClone(defaultSchema)
  schema.tagNames = [
    ...new Set([
      ...(schema.tagNames || []),
      'figure', 'figcaption', 'picture', 'source',
      'video', 'audio', 'track',
      'mark', 'kbd', 'abbr', 'details', 'summary',
      ...(allowedIframeHosts.length ? ['iframe'] : []),
    ]),
  ]
  schema.attributes = {
    ...schema.attributes,
    '*': [...(schema.attributes?.['*'] || []), 'className', 'id'],
    a: [...(schema.attributes?.a || []), 'target', 'rel'],
    img: [...(schema.attributes?.img || []), 'loading', 'decoding', 'width', 'height', 'sizes', 'srcSet'],
    source: ['src', 'srcSet', 'type', 'media', 'sizes'],
    video: ['src', 'poster', 'controls', 'muted', 'loop', 'playsInline', 'preload', 'width', 'height'],
    audio: ['src', 'controls', 'preload'],
    iframe: ['src', 'title', 'allow', 'allowFullScreen', 'loading', 'width', 'height', 'referrerPolicy'],
  }
  return schema
}

// GitHub-style callouts: > [!NOTE] / [!TIP] / [!WARNING] / [!IMPORTANT] / [!CAUTION]
function remarkCallout() {
  const KINDS = new Set(['note', 'tip', 'warning', 'important', 'caution'])
  return (tree) => {
    visit(tree, 'blockquote', (node) => {
      const first = node.children?.[0]
      if (first?.type !== 'paragraph') return
      const text = first.children?.[0]
      if (text?.type !== 'text') return
      const m = /^\[!(\w+)\]\s*\n?/.exec(text.value)
      if (!m) return
      const kind = m[1].toLowerCase()
      if (!KINDS.has(kind)) return
      text.value = text.value.slice(m[0].length)
      if (!text.value) first.children.shift()
      if (!first.children.length) node.children.shift()
      node.data = {
        ...(node.data || {}),
        hName: 'div',
        hProperties: { className: ['callout', `callout-${kind}`], role: 'note' },
      }
    })
  }
}

// An image alone in a paragraph is an argument, not decoration, so it becomes a <figure>.
// The Markdown title (the quoted string after the URL) becomes the <figcaption>.
// See references/product-media.md for the contract this implements.
function rehypeFigure(mediaIssues) {
  const isBlank = (child) => child.type === 'text' && !child.value.trim()
  return (tree) => {
    visit(tree, 'element', (node) => {
      if (node.tagName !== 'p') return
      const kids = (node.children || []).filter((c) => !isBlank(c))
      if (kids.length !== 1) return
      const img = kids[0]
      if (img.type !== 'element' || img.tagName !== 'img') return

      const caption = typeof img.properties?.title === 'string' ? img.properties.title.trim() : ''
      if (caption) delete img.properties.title // the caption is visible; a tooltip would duplicate it

      const alt = typeof img.properties?.alt === 'string' ? img.properties.alt.trim() : ''
      if (!alt) {
        mediaIssues.push({ kind: 'figure-missing-alt', detail: String(img.properties?.src || '(no src)') })
      }

      node.tagName = 'figure'
      node.properties = {}
      node.children = caption
        ? [img, { type: 'element', tagName: 'figcaption', properties: {}, children: [{ type: 'text', value: caption }] }]
        : [img]
    })
  }
}

// Record what sanitization is about to strip, so validation can name the file and
// the tag instead of silently dropping content the author believed had shipped.
function collectUnsafe(report, schema, allowedIframeHosts) {
  const allowed = new Set(schema.tagNames)
  return (tree) => {
    visit(tree, 'element', (node) => {
      if (node.tagName === 'iframe') {
        const src = String(node.properties?.src || '')
        let host = null
        try { host = new URL(src, 'https://placeholder.invalid').hostname } catch { host = null }
        if (!allowedIframeHosts.includes(host)) {
          report.push({ kind: 'iframe-host', detail: host || src })
          node.tagName = 'div'
          node.properties = {}
          node.children = []
          return
        }
        node.properties = {
          ...node.properties,
          loading: 'lazy',
          referrerPolicy: 'no-referrer-when-downgrade',
        }
        return
      }
      if (!allowed.has(node.tagName)) {
        report.push({ kind: 'tag', detail: node.tagName })
      }
      for (const key of Object.keys(node.properties || {})) {
        if (EVENT_ATTR.test(key) && key.length > 2) report.push({ kind: 'event-attr', detail: key })
      }
      for (const key of ['href', 'src', 'action', 'formAction']) {
        const value = node.properties?.[key]
        if (typeof value === 'string' && DANGEROUS_URL.test(value)) {
          report.push({ kind: 'url-protocol', detail: value.slice(0, 60) })
        }
      }
    })
  }
}

function rehypeEnhance({ toc, links, images, siteUrl }) {
  return (tree) => {
    visit(tree, 'element', (node) => {
      if (/^h[23]$/.test(node.tagName) && node.properties?.id) {
        toc.push({ depth: Number(node.tagName[1]), id: String(node.properties.id), text: hastToString(node) })
      }
      if (node.tagName === 'h1') toc.h1 = true
      if (node.tagName === 'a') {
        const href = String(node.properties?.href || '')
        links.push(href)
        const external = /^https?:\/\//i.test(href) && !href.startsWith(siteUrl)
        if (external) {
          node.properties.target = '_blank'
          node.properties.rel = 'noopener noreferrer'
        }
      }
      if (node.tagName === 'img') {
        images.push(String(node.properties?.src || ''))
        node.properties.loading = node.properties.loading || 'lazy'
        node.properties.decoding = node.properties.decoding || 'async'
      }
    })
  }
}

/**
 * Render an article body to HTML plus its derived data.
 * @returns {{html:string, toc:Array, plain:string, links:string[], images:string[], unsafe:Array, mediaIssues:Array, hasH1:boolean}}
 */
export function renderArticle(source, { allowedIframeHosts = [], siteUrl = '' } = {}) {
  const schema = buildSchema({ allowedIframeHosts })
  const unsafe = []
  const mediaIssues = []
  const toc = []
  const links = []
  const images = []

  const file = unified()
    .use(remarkParse)
    .use(remarkGfm)
    .use(remarkCallout)
    .use(remarkRehype, { allowDangerousHtml: true })
    .use(rehypeRaw)
    .use(rehypeFigure, mediaIssues)
    .use(collectUnsafe, unsafe, schema, allowedIframeHosts)
    .use(rehypeSanitize, schema)
    .use(rehypeSlug)
    .use(rehypeEnhance, { toc, links, images, siteUrl })
    .use(rehypeStringify, { allowDangerousHtml: false })
    .processSync(source)

  const html = String(file)
  const plain = html.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ').trim()
  const hasH1 = Boolean(toc.h1)
  delete toc.h1

  return { html, toc: [...toc], plain, links, images, unsafe, mediaIssues, hasH1 }
}
