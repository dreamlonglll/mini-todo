import { describe, expect, it } from 'vitest'
import { isImeComposing, resolveEditorShortcut } from './editorShortcuts'

function key(
  init: Partial<
    Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey' | 'isComposing' | 'keyCode'>
  >
) {
  return {
    key: '',
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    shiftKey: false,
    isComposing: false,
    keyCode: 0,
    ...init,
  }
}

describe('resolveEditorShortcut', () => {
  it('maps Esc to close and Ctrl/Cmd+Enter to submit', () => {
    expect(resolveEditorShortcut(key({ key: 'Escape', keyCode: 27 }))).toBe('close')
    expect(resolveEditorShortcut(key({ key: 'Enter', ctrlKey: true, keyCode: 13 }))).toBe('submit')
    expect(resolveEditorShortcut(key({ key: 'Enter', metaKey: true, keyCode: 13 }))).toBe('submit')
  })

  it('ignores plain Enter and modified Esc', () => {
    expect(resolveEditorShortcut(key({ key: 'Enter', keyCode: 13 }))).toBeNull()
    expect(resolveEditorShortcut(key({ key: 'Escape', ctrlKey: true }))).toBeNull()
    expect(resolveEditorShortcut(key({ key: 'Enter', ctrlKey: true, shiftKey: true }))).toBeNull()
    expect(resolveEditorShortcut(key({ key: 'Enter', ctrlKey: true, altKey: true }))).toBeNull()
    expect(resolveEditorShortcut(key({ key: 's', ctrlKey: true }))).toBeNull()
  })

  it('leaves keys to the IME while composing', () => {
    expect(resolveEditorShortcut(key({ key: 'Escape', isComposing: true }))).toBeNull()
    expect(resolveEditorShortcut(key({ key: 'Enter', ctrlKey: true, isComposing: true }))).toBeNull()
    // 部分 WebView 组合输入时只给 keyCode 229
    expect(resolveEditorShortcut(key({ key: 'Enter', ctrlKey: true, keyCode: 229 }))).toBeNull()
  })
})

describe('isImeComposing', () => {
  it('detects composition from either signal', () => {
    expect(isImeComposing({ isComposing: true, keyCode: 13 })).toBe(true)
    expect(isImeComposing({ isComposing: false, keyCode: 229 })).toBe(true)
    expect(isImeComposing({ isComposing: false, keyCode: 13 })).toBe(false)
  })
})
