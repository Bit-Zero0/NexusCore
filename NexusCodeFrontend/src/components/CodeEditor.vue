<script setup lang="ts">
import { autocompletion, closeBrackets, closeBracketsKeymap } from '@codemirror/autocomplete'
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
import { cpp } from '@codemirror/lang-cpp'
import {
  defaultHighlightStyle,
  foldGutter,
  indentUnit,
  indentOnInput,
  syntaxHighlighting,
} from '@codemirror/language'
import { python } from '@codemirror/lang-python'
import { searchKeymap } from '@codemirror/search'
import { Compartment, EditorState } from '@codemirror/state'
import {
  drawSelection,
  dropCursor,
  EditorView,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
} from '@codemirror/view'
import { oneDark, oneDarkHighlightStyle } from '@codemirror/theme-one-dark'
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'

const props = withDefaults(
  defineProps<{
    modelValue: string
    language: 'cpp' | 'python'
    lineNumbers?: boolean
    fontSize?: number
    tabSize?: number
    readOnly?: boolean
  }>(),
  {
    lineNumbers: true,
    fontSize: 15,
    tabSize: 4,
    readOnly: false,
  },
)

const emit = defineEmits<{
  'update:modelValue': [value: string]
}>()

const hostRef = ref<HTMLDivElement | null>(null)
let editorView: EditorView | null = null
let themeObserver: MutationObserver | null = null

const languageCompartment = new Compartment()
const lineNumberCompartment = new Compartment()
const appearanceCompartment = new Compartment()
const indentationCompartment = new Compartment()
const readOnlyCompartment = new Compartment()

const keymaps = keymap.of([
  indentWithTab,
  ...defaultKeymap,
  ...historyKeymap,
  ...closeBracketsKeymap,
  ...searchKeymap,
])

const getLanguageExtension = (language: 'cpp' | 'python') =>
  language === 'python' ? python() : cpp()

const getLineNumberExtension = (visible: boolean) =>
  visible ? [lineNumbers(), highlightActiveLineGutter(), foldGutter()] : []

const getIndentationExtension = () => [
  EditorState.tabSize.of(props.tabSize),
  indentUnit.of(' '.repeat(props.tabSize)),
]

const getReadOnlyExtension = () => [
  EditorState.readOnly.of(props.readOnly),
  EditorView.editable.of(!props.readOnly),
]

const getAppearanceExtension = () => {
  const fontSize = `${props.fontSize}px`
  const baseTheme = EditorView.theme(
    {
      '&': {
        height: '100%',
        backgroundColor: 'transparent',
        color: 'var(--text)',
        fontFamily: 'var(--font-code)',
        fontSize,
      },
      '.cm-scroller': {
        overflow: 'auto',
        fontFamily: 'var(--font-code)',
        lineHeight: '1.7',
        paddingTop: '20px',
        paddingBottom: '24px',
      },
      '.cm-content': {
        minHeight: '100%',
        paddingRight: '24px',
        paddingLeft: '8px',
      },
      '.cm-line': {
        padding: '0',
      },
      '.cm-gutters': {
        backgroundColor: 'rgba(117, 81, 36, 0.06)',
        borderRight: '1px solid var(--border)',
        color: 'var(--text-mute)',
      },
      '.cm-gutter': {
        minHeight: '100%',
        fontFamily: 'var(--font-code)',
        fontSize,
      },
      '.cm-gutterElement': {
        height: '1.7em',
        lineHeight: '1.7',
        display: 'flex',
        alignItems: 'center',
      },
      '.cm-lineNumbers .cm-gutterElement': {
        minWidth: '2.5ch',
        paddingRight: '12px',
        textAlign: 'right',
        justifyContent: 'flex-end',
      },
      '.cm-foldGutter .cm-gutterElement': {
        cursor: 'pointer',
        justifyContent: 'center',
      },
      '.cm-activeLine, .cm-activeLineGutter': {
        backgroundColor: 'rgba(240, 138, 36, 0.08)',
      },
      '.cm-selectionBackground, ::selection': {
        backgroundColor: 'rgba(240, 138, 36, 0.2) !important',
      },
      '.cm-cursor, .cm-dropCursor': {
        borderLeftColor: 'var(--accent)',
      },
      '.cm-focused': {
        outline: 'none',
      },
    },
    { dark: false },
  )

  const lightTheme = EditorView.theme(
    {
      '.cm-gutters': {
        backgroundColor: 'rgba(117, 81, 36, 0.06)',
      },
    },
    { dark: false },
  )

  return document.documentElement.dataset.theme === 'dark'
    ? [oneDark, baseTheme, syntaxHighlighting(oneDarkHighlightStyle)]
    : [baseTheme, lightTheme, syntaxHighlighting(defaultHighlightStyle, { fallback: true })]
}

const syncExternalValue = (nextValue: string) => {
  if (!editorView) return
  const currentValue = editorView.state.doc.toString()
  if (currentValue === nextValue) return

  editorView.dispatch({
    changes: {
      from: 0,
      to: currentValue.length,
      insert: nextValue,
    },
  })
}

const buildEditorState = () =>
  EditorState.create({
    doc: props.modelValue,
    extensions: [
      history(),
      drawSelection(),
      dropCursor(),
      indentOnInput(),
      closeBrackets(),
      autocompletion(),
      highlightActiveLine(),
      keymaps,
      languageCompartment.of(getLanguageExtension(props.language)),
      lineNumberCompartment.of(getLineNumberExtension(props.lineNumbers)),
      indentationCompartment.of(getIndentationExtension()),
      readOnlyCompartment.of(getReadOnlyExtension()),
      appearanceCompartment.of(getAppearanceExtension()),
      EditorView.updateListener.of((update) => {
        if (update.docChanged) {
          emit('update:modelValue', update.state.doc.toString())
        }
      }),
    ],
  })

const refreshAppearance = () => {
  if (!editorView) return
  editorView.dispatch({
    effects: appearanceCompartment.reconfigure(getAppearanceExtension()),
  })
}

onMounted(() => {
  if (!hostRef.value) return
  editorView = new EditorView({
    state: buildEditorState(),
    parent: hostRef.value,
  })

  themeObserver = new MutationObserver(() => {
    refreshAppearance()
  })
  themeObserver.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['data-theme'],
  })
})

watch(
  () => props.modelValue,
  (nextValue) => {
    syncExternalValue(nextValue)
  },
)

watch(
  () => props.language,
  (nextLanguage) => {
    if (!editorView) return
    editorView.dispatch({
      effects: languageCompartment.reconfigure(getLanguageExtension(nextLanguage)),
    })
  },
)

watch(
  () => props.lineNumbers,
  (visible) => {
    if (!editorView) return
    editorView.dispatch({
      effects: lineNumberCompartment.reconfigure(getLineNumberExtension(visible)),
    })
  },
)

watch(
  () => props.fontSize,
  () => {
    refreshAppearance()
  },
)

watch(
  () => props.tabSize,
  () => {
    if (!editorView) return
    editorView.dispatch({
      effects: indentationCompartment.reconfigure(getIndentationExtension()),
    })
  },
)

watch(
  () => props.readOnly,
  () => {
    if (!editorView) return
    editorView.dispatch({
      effects: readOnlyCompartment.reconfigure(getReadOnlyExtension()),
    })
  },
)

onBeforeUnmount(() => {
  themeObserver?.disconnect()
  editorView?.destroy()
  themeObserver = null
  editorView = null
})
</script>

<template>
  <div ref="hostRef" class="code-editor-shell"></div>
</template>

<style scoped>
.code-editor-shell {
  height: 100%;
  min-height: 0;
  background:
    linear-gradient(180deg, rgba(255, 250, 244, 0.96), rgba(252, 248, 241, 0.94));
}

:global(:root[data-theme='dark']) .code-editor-shell {
  background:
    linear-gradient(180deg, rgba(12, 18, 31, 0.96), rgba(11, 16, 28, 0.94));
}

.code-editor-shell :deep(.cm-editor) {
  height: 100%;
}

.code-editor-shell :deep(.cm-scroller) {
  overflow: auto;
}

.code-editor-shell :deep(.cm-content),
.code-editor-shell :deep(.cm-gutter) {
  font-family: var(--font-code);
}
</style>
