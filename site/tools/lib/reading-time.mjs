// Reading time.
// Applying an English words-per-minute constant to a Chinese site reports a
// 3,000-character article as one minute, so CJK characters and Latin words are
// counted separately and summed.
const CJK = /[㐀-䶿一-鿿豈-﫿぀-ヿ가-힯]/gu

export function readingMinutes(plainText, opts = {}) {
  const { cjkCharsPerMinute = 350, wordsPerMinute = 220, minMinutes = 1 } = opts
  const text = String(plainText || '')
  const cjkCount = (text.match(CJK) || []).length
  const nonCjk = text.replace(CJK, ' ')
  const wordCount = (nonCjk.match(/[A-Za-z0-9][A-Za-z0-9'’\-]*/g) || []).length
  const minutes = cjkCount / cjkCharsPerMinute + wordCount / wordsPerMinute
  return Math.max(minMinutes, Math.round(minutes) || minMinutes)
}

export function countText(plainText) {
  const text = String(plainText || '')
  const cjkCount = (text.match(CJK) || []).length
  const wordCount = (text.replace(CJK, ' ').match(/[A-Za-z0-9][A-Za-z0-9'’\-]*/g) || []).length
  return { cjkCount, wordCount, total: cjkCount + wordCount }
}
