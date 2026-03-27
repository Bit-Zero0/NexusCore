import MarkdownIt from 'markdown-it'

export interface MarkdownHeading {
  id: string
  title: string
  level: number
}

const markdown = new MarkdownIt({
  html: false,
  breaks: true,
  linkify: true,
  typographer: true,
})

function slugify(value: string): string {
  return value
    .toLowerCase()
    .trim()
    .replace(/[^\w\u4e00-\u9fa5\s-]/g, '')
    .replace(/\s+/g, '-')
    .replace(/-+/g, '-')
}

export function renderMarkdown(source?: string | null): string {
  if (!source || !source.trim()) {
    return '<p>暂无内容</p>'
  }

  return markdown.render(source)
}

export function renderMarkdownDocument(source?: string | null): {
  html: string
  headings: MarkdownHeading[]
} {
  if (!source || !source.trim()) {
    return {
      html: '<p>暂无内容</p>',
      headings: [],
    }
  }

  const tokens = markdown.parse(source, {})
  const headings: MarkdownHeading[] = []
  const usedIds = new Set<string>()

  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index]
    if (!token || token.type !== 'heading_open') continue

    const inlineToken = tokens[index + 1]
    const title = inlineToken?.content?.trim() || 'section'
    const level = Number(token.tag.replace('h', '')) || 2
    let id = slugify(title) || `section-${index}`

    if (usedIds.has(id)) {
      let suffix = 2
      while (usedIds.has(`${id}-${suffix}`)) suffix += 1
      id = `${id}-${suffix}`
    }
    usedIds.add(id)
    token.attrSet('id', id)
    headings.push({ id, title, level })
  }

  return {
    html: markdown.renderer.render(tokens, markdown.options, {}),
    headings,
  }
}
