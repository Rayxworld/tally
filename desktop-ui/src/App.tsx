import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { invoke } from '@tauri-apps/api/core'
import {
  AlertCircle, ArrowDown, ArrowLeft, ArrowRight, ArrowUp, Check,
  ChevronRight, Code2, Copy, ExternalLink, Eye, File, FileArchive,
  FileAudio, FileImage, FileText, Film, Folder, FolderOpen, FolderPlus,
  HardDrive, History, Home, Image, LayoutGrid, List,
  Monitor, Moon, Music2, PanelLeftClose, Plus, Radio,
  RefreshCw, Search, Settings, SlidersHorizontal, Sparkles, Sun,
  Tag, Trash2, Undo2, X, Zap, CircleHelp
} from 'lucide-react'
import './App.css'

// ─── Types ────────────────────────────────────────────────────────────────────

type NativeFileEntry = {
  name: string
  path: string
  is_dir: boolean
  extension: string | null
  size: number
  modified: number | null
}

type NativeDirectory = {
  path: string
  parent: string | null
  entries: NativeFileEntry[]
}

type SpecialLocation = {
  name: string
  path: string
  is_drive: boolean
}

type FileItem = {
  name: string
  kind: 'folder' | 'image' | 'document' | 'video' | 'audio' | 'code' | 'archive' | 'other'
  modified: string
  size: string
  path: string
  isDir: boolean
  bytes: number
  modifiedTimestamp: number | null
}

type PlannedAction = {
  source: string
  destination: string | null
  kind: 'Move' | 'Duplicate' | 'Skip' | 'Failed'
  reason: string
  rule_name: string | null
}

type SweepSummary = {
  moved: number
  duplicates: number
  skipped: number
  failed: number
}

type SweepPlan = {
  actions: PlannedAction[]
  summary: SweepSummary
  warning: string | null
}

type Transaction = {
  transaction_id: string
  timestamp: string
  target: string
  operations: { source_path: string; destination_path: string; action: string }[]
  created_dirs: string[]
}

type Rule = { name: string; destination: string; extensions: string[] }

type Config = {
  version: number
  min_age_seconds: number
  duplicates_folder: string
  skip_extensions: string[]
  skip_names: string[]
  rules: Rule[]
}

type TabState = {
  path: string
  name: string
  history: string[]
  historyIndex: number
}

type FilePreview = {
  kind: 'image' | 'text' | 'binary'
  content: string
  line_count?: number
}

type DuplicateFile = {
  path: string
  name: string
  size: number
  modified: number | null
}

type DuplicateGroup = {
  hash: string
  size: number
  wasted_bytes: number
  files: DuplicateFile[]
}

type StorageCategory = {
  label: string
  count: number
  total_bytes: number
  color: string
}

type StorageBreakdown = {
  total_bytes: number
  file_count: number
  dir_count: number
  top_files: NativeFileEntry[]
  categories: StorageCategory[]
}

// ─── Tag colour definitions ────────────────────────────────────────────────────

const TAG_COLOURS: Record<string, string> = {
  red:    '#ef4444',
  orange: '#f97316',
  yellow: '#eab308',
  green:  '#22c55e',
  blue:   '#3b82f6',
  purple: '#a855f7',
}

// ─── Command palette entries ───────────────────────────────────────────────────

type PaletteAction = {
  id: string
  label: string
  description?: string
  run: () => void
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

function FileGlyph({ item, large = false }: { item: FileItem; large?: boolean }) {
  const sz = large ? 32 : 18
  const sw = 1.8
  if (item.kind === 'folder')   return <Folder      size={sz} strokeWidth={sw} className="folder-glyph" />
  if (item.kind === 'image')    return <FileImage    size={sz} strokeWidth={sw} className="image-glyph" />
  if (item.kind === 'document') return <FileText     size={sz} strokeWidth={sw} className="doc-glyph" />
  if (item.kind === 'video')    return <Film         size={sz} strokeWidth={sw} className="video-glyph" />
  if (item.kind === 'audio')    return <FileAudio    size={sz} strokeWidth={sw} className="audio-glyph" />
  if (item.kind === 'code')     return <Code2        size={sz} strokeWidth={sw} className="code-glyph" />
  if (item.kind === 'archive')  return <FileArchive  size={sz} strokeWidth={sw} className="archive-glyph" />
  return <File size={sz} strokeWidth={sw} />
}

function kindFromEntry(entry: NativeFileEntry): FileItem['kind'] {
  if (entry.is_dir) return 'folder'
  const ext = (entry.extension ?? '').toLowerCase()
  if (['png','jpg','jpeg','gif','webp','bmp','svg','ico','avif','tif','tiff'].includes(ext)) return 'image'
  if (['mp4','mkv','mov','avi','webm','wmv','m4v'].includes(ext)) return 'video'
  if (['mp3','wav','flac','aac','ogg','m4a','wma'].includes(ext)) return 'audio'
  if (['zip','7z','rar','tar','gz','bz2','xz','zst'].includes(ext)) return 'archive'
  if (['js','jsx','ts','tsx','html','css','scss','json','py','rs','java','c','cpp','h','go','php','sql','sh','ps1','md','yml','yaml','toml','xml'].includes(ext)) return 'code'
  if (['pdf','doc','docx','txt','rtf','odt','xls','xlsx','ppt','pptx','csv','tsv'].includes(ext)) return 'document'
  return 'other'
}

function fmtBytes(bytes: number, isDir: boolean): string {
  if (isDir) return '—'
  if (bytes < 1024) return `${bytes} B`
  const units = ['KB','MB','GB','TB']
  let v = bytes / 1024, u = 0
  while (v >= 1024 && u < units.length - 1) { v /= 1024; u++ }
  return `${v.toFixed(v >= 100 ? 0 : v >= 10 ? 1 : 2)} ${units[u]}`
}

function fmtModified(ts: number | null): string {
  if (ts == null) return '—'
  return new Date(ts * 1000).toLocaleString(undefined, {
    year: 'numeric', month: 'short', day: 'numeric',
    hour: '2-digit', minute: '2-digit',
  })
}

function basename(path: string): string {
  const parts = path.split(/[/\\]/).filter(Boolean)
  return parts.length ? parts[parts.length - 1] : path
}

function humanBytes(b: number): string {
  if (b < 1024) return `${b} B`
  const u = ['KB','MB','GB','TB']
  let v = b / 1024, i = 0
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++ }
  return `${v.toFixed(1)} ${u[i]}`
}

// ─── App ──────────────────────────────────────────────────────────────────────

export default function App() {
  // ── Navigation state ────────────────────────────────────────────────────────
  const [tabs, setTabs] = useState<TabState[]>([])
  const [activeTab, setActiveTab] = useState(0)
  const [specialLocations, setSpecialLocations] = useState<SpecialLocation[]>([])
  const [directory, setDirectory] = useState<NativeDirectory | null>(null)
  const [loading, setLoading] = useState(false)
  const [loadError, setLoadError] = useState('')
  const [editingPath, setEditingPath] = useState(false)
  const [pathInput, setPathInput] = useState('')
  const pathInputRef = useRef<HTMLInputElement>(null)

  // ── File list state ──────────────────────────────────────────────────────────
  const [selected, setSelected] = useState<string[]>([])
  const [query, setQuery] = useState('')
  const [sortBy, setSortBy] = useState<'name' | 'modified' | 'size' | 'type'>('name')
  const [sortAsc, setSortAsc] = useState(true)
  const [view, setView] = useState<'list' | 'grid'>('list')
  const [categoryFilter, setCategoryFilter] = useState('all')

  // ── UI state ─────────────────────────────────────────────────────────────────
  const [dark, setDark] = useState(true)
  const [showDetails, setShowDetails] = useState(true)
  const [toast, setToast] = useState('')
  const [previewData, setPreviewData] = useState<FilePreview | null>(null)
  const [previewLoading, setPreviewLoading] = useState(false)
  const [isDraggingOver, setIsDraggingOver] = useState(false)
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; item: FileItem } | null>(null)
  const [routeItem, setRouteItem] = useState<FileItem | null>(null)
  const [routeDestination, setRouteDestination] = useState('{documents}/Other')
  const [routeLabel, setRouteLabel] = useState('My file types')

  // ── Watch / tally state ──────────────────────────────────────────────────────
  const [watchedFolders, setWatchedFolders] = useState<string[]>([])
  const [contextMenuEnabled, setContextMenuEnabled] = useState(true)

  // ── Sweep panel ──────────────────────────────────────────────────────────────
  const [showSweepPanel, setShowSweepPanel] = useState(false)
  const [sweepPlan, setSweepPlan] = useState<SweepPlan | null>(null)
  const [sweepLoading, setSweepLoading] = useState(false)
  const [sweepResult, setSweepResult] = useState<SweepSummary | null>(null)

  // ── History modal ─────────────────────────────────────────────────────────────
  const [showHistoryModal, setShowHistoryModal] = useState(false)
  const [historyList, setHistoryList] = useState<Transaction[]>([])

  // ── Config modal ──────────────────────────────────────────────────────────────
  const [showConfigModal, setShowConfigModal] = useState(false)
  const [activeConfig, setActiveConfig] = useState<Config | null>(null)

  // ── New folder modal ──────────────────────────────────────────────────────────
  const [showNewFolderModal, setShowNewFolderModal] = useState(false)
  const [newFolderName, setNewFolderName] = useState('')

  // ── Command palette (Ctrl+K) ──────────────────────────────────────────────────
  const [showPalette, setShowPalette] = useState(false)
  const [showHelp, setShowHelp] = useState(false)
  const [paletteQuery, setPaletteQuery] = useState('')
  const paletteInputRef = useRef<HTMLInputElement>(null)

  // ── Duplicate scanner ─────────────────────────────────────────────────────────
  const [showDupeModal, setShowDupeModal] = useState(false)
  const [dupeGroups, setDupeGroups] = useState<DuplicateGroup[]>([])
  const [dupeLoading, setDupeLoading] = useState(false)
  const [dupeSelected, setDupeSelected] = useState<Set<string>>(new Set())

  // ── Storage visualizer ────────────────────────────────────────────────────────
  const [showStorageModal, setShowStorageModal] = useState(false)
  const [storageData, setStorageData] = useState<StorageBreakdown | null>(null)
  const [storageLoading, setStorageLoading] = useState(false)

  // ── Batch rename ──────────────────────────────────────────────────────────────
  const [showRenameModal, setShowRenameModal] = useState(false)
  const [renameItems, setRenameItems] = useState<{ from: string; name: string; to: string }[]>([])
  const [renamePrefix, setRenamePrefix] = useState('')
  const [renameSuffix, setRenameSuffix] = useState('')
  const [renameFindVal, setRenameFindVal] = useState('')
  const [renameReplaceVal, setRenameReplaceVal] = useState('')
  const [renameNumberStart, setRenameNumberStart] = useState('')

  // ── Tags ──────────────────────────────────────────────────────────────────────
  const [fileTags, setFileTags] = useState<Record<string, string>>({})

  // ─────────────────────────────────────────────────────────────────────────────

  const currentTab   = tabs[activeTab] || null
  const currentPath  = currentTab?.path ?? null

  // ── Init ─────────────────────────────────────────────────────────────────────
  useEffect(() => {
    invoke<SpecialLocation[]>('get_special_locations').then(locs => {
      setSpecialLocations(locs)
      const home = locs.find(l => l.name === 'Home') || locs[0]
      if (home) {
        setTabs([{ path: home.path, name: home.name, history: [home.path], historyIndex: 0 }])
        setActiveTab(0)
      }
    }).catch(err => setLoadError(`Failed to init filesystem: ${err}`))

    invoke<string[]>('get_watched_folders').then(setWatchedFolders).catch(() => {})
    invoke<Record<string,string>>('get_file_tags').then(setFileTags).catch(() => {})
  }, [])

  // ── Load directory ────────────────────────────────────────────────────────────
  useEffect(() => {
    if (!currentPath) return
    setLoading(true)
    setLoadError('')
    setSelected([])
    setContextMenu(null)
    setPreviewData(null)

    invoke<NativeDirectory>('list_directory', { path: currentPath })
      .then(res => { setDirectory(res); setLoading(false) })
      .catch(err => { setDirectory(null); setLoadError(String(err)); setLoading(false) })
  }, [currentPath])

  // ── Preview on single selection ───────────────────────────────────────────────
  useEffect(() => {
    if (selected.length !== 1) { setPreviewData(null); return }
    const entry = directory?.entries.find(e => e.path === selected[0])
    if (!entry || entry.is_dir) { setPreviewData(null); return }

    setPreviewLoading(true)
    invoke<FilePreview>('get_file_preview', { path: selected[0] })
      .then(p => { setPreviewData(p); setPreviewLoading(false) })
      .catch(() => { setPreviewData(null); setPreviewLoading(false) })
  }, [selected, directory])

  // ── Global Ctrl+K ─────────────────────────────────────────────────────────────
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === 'k') {
        e.preventDefault()
        setPaletteQuery('')
        setShowPalette(p => !p)
      }
      if (e.key === 'Escape') {
        setShowPalette(false)
        setContextMenu(null)
        setShowHelp(false)
      }
      if ((e.ctrlKey || e.metaKey) && e.key === '/') { e.preventDefault(); setShowHelp(true) }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  useEffect(() => {
    if (showPalette) setTimeout(() => paletteInputRef.current?.focus(), 20)
  }, [showPalette])

  // ── Close context menu on outside click ───────────────────────────────────────
  useEffect(() => {
    const h = () => setContextMenu(null)
    window.addEventListener('click', h)
    return () => window.removeEventListener('click', h)
  }, [])

  // ─── Helpers ──────────────────────────────────────────────────────────────────

  function notify(msg: string) {
    setToast(msg)
    window.setTimeout(() => setToast(''), 3200)
  }

  function navigateTo(path: string, customName?: string) {
    if (!currentTab) return
    setTabs(prev => {
      const next = [...prev]
      const tab = { ...next[activeTab] }
      const hist = tab.history.slice(0, tab.historyIndex + 1)
      hist.push(path)
      tab.history = hist
      tab.historyIndex = hist.length - 1
      tab.path = path
      tab.name = customName || basename(path)
      next[activeTab] = tab
      return next
    })
  }

  function goBack() {
    if (!currentTab || currentTab.historyIndex <= 0) return
    const ni = currentTab.historyIndex - 1
    setTabs(prev => {
      const next = [...prev]
      next[activeTab] = { ...next[activeTab], path: next[activeTab].history[ni], name: basename(next[activeTab].history[ni]), historyIndex: ni }
      return next
    })
  }

  function goForward() {
    if (!currentTab || currentTab.historyIndex >= currentTab.history.length - 1) return
    const ni = currentTab.historyIndex + 1
    setTabs(prev => {
      const next = [...prev]
      next[activeTab] = { ...next[activeTab], path: next[activeTab].history[ni], name: basename(next[activeTab].history[ni]), historyIndex: ni }
      return next
    })
  }

  function goUp() { if (directory?.parent) navigateTo(directory.parent) }

  function refresh() {
    if (!currentPath) return
    setLoading(true)
    invoke<NativeDirectory>('list_directory', { path: currentPath })
      .then(res => { setDirectory(res); setLoading(false) })
      .catch(err => { setLoadError(String(err)); setLoading(false) })
  }

  function openNewTab(path?: string) {
    const p = path || currentTab?.path || specialLocations[0]?.path || 'C:\\'
    setTabs(prev => [...prev, { path: p, name: basename(p), history: [p], historyIndex: 0 }])
    setActiveTab(tabs.length)
  }

  function closeTab(idx: number, e?: React.MouseEvent) {
    e?.stopPropagation()
    if (tabs.length <= 1) return
    const next = tabs.filter((_, i) => i !== idx)
    setTabs(next)
    setActiveTab(Math.min(activeTab, next.length - 1))
  }

  // ─── File operations ──────────────────────────────────────────────────────────

  function handleDoubleClick(item: FileItem) {
    if (item.isDir) { navigateTo(item.path, item.name); return }
    invoke('open_path', { path: item.path })
      .then(() => notify(`Opened ${item.name}`))
      .catch(err => notify(`Error: ${err}`))
  }

  function toggleSelect(path: string, e: React.MouseEvent) {
    if (e.ctrlKey || e.metaKey) {
      setSelected(p => p.includes(path) ? p.filter(x => x !== path) : [...p, path])
    } else if (e.shiftKey && selected.length > 0) {
      const last = selected[selected.length - 1]
      const li = filteredFiles.findIndex(f => f.path === last)
      const ci = filteredFiles.findIndex(f => f.path === path)
      if (li !== -1 && ci !== -1) {
        const [s, en] = [Math.min(li, ci), Math.max(li, ci)]
        setSelected(Array.from(new Set([...selected, ...filteredFiles.slice(s, en + 1).map(f => f.path)])))
      }
    } else {
      setSelected([path])
    }
  }

  function handleContextMenu(e: React.MouseEvent, item: FileItem) {
    e.preventDefault(); e.stopPropagation()
    setSelected([item.path])
    setContextMenu({ x: e.clientX, y: e.clientY, item })
  }

  function extractArchive(item: FileItem) {
    invoke<string>('extract_archive', { path: item.path })
      .then(folder => { notify(`Extracted to ${basename(folder)}`); refresh() })
      .catch(err => notify(`Extraction error: ${err}`))
  }

  function teachTally(item: FileItem) {
    const extension = item.name.includes('.') ? `.${item.name.split('.').pop()}` : ''
    if (!extension) { notify('This file has no extension to teach Tally.'); return }
    setRouteItem(item); setRouteDestination('{documents}/Other'); setRouteLabel('My file types')
  }

  function saveFileType() {
    if (!routeItem) return
    const extension = `.${routeItem.name.split('.').pop()}`
    invoke<Config>('assign_extension', { extension, destination: routeDestination, label: routeLabel })
      .then(config => { setActiveConfig(config); notify(`Tally will now sort ${extension} files to ${routeDestination}`); setRouteItem(null) })
      .catch(err => notify(`Could not save file type: ${err}`))
  }

  function handleCreateFolder() {
    if (!currentPath || !newFolderName.trim()) return
    invoke<string>('create_directory', { parentPath: currentPath, name: newFolderName.trim() })
      .then(p => { setShowNewFolderModal(false); setNewFolderName(''); notify(`Created folder: ${basename(p)}`); refresh() })
      .catch(err => notify(`Error: ${err}`))
  }

  function handleDeleteSelected() {
    if (!selected.length) return
    if (!window.confirm(`Delete ${selected.length} item(s)? This cannot be undone.`)) return
    Promise.all(selected.map(p => invoke('delete_path', { path: p })))
      .then(() => { notify(`Deleted ${selected.length} item(s)`); setSelected([]); refresh() })
      .catch(err => notify(`Error deleting: ${err}`))
  }

  // ─── Tally sweep ──────────────────────────────────────────────────────────────

  function openSweepPreview() {
    if (!currentPath) return
    setSweepLoading(true); setSweepResult(null); setShowSweepPanel(true)
    invoke<SweepPlan>('get_tally_plan', { path: currentPath })
      .then(plan => { setSweepPlan(plan); setSweepLoading(false) })
      .catch(err => { notify(`Cannot plan: ${err}`); setSweepLoading(false) })
  }

  function runSweep() {
    if (!currentPath) return
    setSweepLoading(true)
    invoke<SweepSummary>('execute_tally_sweep', { path: currentPath })
      .then(s => {
        setSweepResult(s); setSweepLoading(false)
        notify(`Sweep done: ${s.moved} moved, ${s.duplicates} duplicates preserved`)
        refresh()
      })
      .catch(err => { notify(`Sweep failed: ${err}`); setSweepLoading(false) })
  }

  function handleUndo(count = 1) {
    invoke<{ restored: number; missing: number; failed: number }>('undo_tally_sweep', { count })
      .then(r => { notify(`Undo: ${r.restored} restored, ${r.failed} failed`); refresh() })
      .catch(err => notify(`Undo failed: ${err}`))
  }

  function openHistory() {
    invoke<Transaction[]>('get_tally_history')
      .then(txs => { setHistoryList([...txs].reverse()); setShowHistoryModal(true) })
      .catch(err => notify(`Cannot load history: ${err}`))
  }

  function openConfig() {
    invoke<Config>('get_tally_config')
      .then(cfg => { setActiveConfig(cfg); setShowConfigModal(true) })
      .catch(err => notify(`Cannot load config: ${err}`))
  }

  function toggleWatchFolder() {
    if (!currentPath) return
    const watching = watchedFolders.includes(currentPath)
    invoke<string[]>('toggle_watch_folder', { path: currentPath, enable: !watching })
      .then(f => { setWatchedFolders(f); notify(watching ? 'Stopped watching' : 'Auto-clean enabled for this folder') })
      .catch(err => notify(`Watch error: ${err}`))
  }

  function toggleContextMenuIntegration() {
    const next = !contextMenuEnabled
    invoke('toggle_windows_context_menu', { enable: next })
      .then(() => { setContextMenuEnabled(next); notify(next ? 'Added to Windows Explorer menu' : 'Removed from Explorer menu') })
      .catch(err => notify(`Error: ${err}`))
  }

  // ─── Duplicate scanner ────────────────────────────────────────────────────────

  function openDupeScanner() {
    if (!currentPath) return
    setDupeLoading(true); setDupeGroups([]); setDupeSelected(new Set()); setShowDupeModal(true)
    invoke<DuplicateGroup[]>('scan_duplicates', { path: currentPath })
      .then(g => { setDupeGroups(g); setDupeLoading(false) })
      .catch(err => { notify(`Scan failed: ${err}`); setDupeLoading(false) })
  }

  function toggleDupeFile(path: string) {
    setDupeSelected(prev => {
      const next = new Set(prev)
      if (next.has(path)) next.delete(path); else next.add(path)
      return next
    })
  }

  function isolateSelectedDupes() {
    if (!currentPath || dupeSelected.size === 0) return
    invoke<number>('isolate_duplicates', { folderPath: currentPath, duplicatePaths: Array.from(dupeSelected) })
      .then(n => {
        notify(`Moved ${n} duplicate(s) to _Duplicates/`)
        setShowDupeModal(false); setDupeSelected(new Set()); refresh()
      })
      .catch(err => notify(`Error: ${err}`))
  }

  // ─── Storage visualizer ───────────────────────────────────────────────────────

  function openStorage() {
    if (!currentPath) return
    setStorageLoading(true); setStorageData(null); setShowStorageModal(true)
    invoke<StorageBreakdown>('get_folder_storage', { path: currentPath })
      .then(d => { setStorageData(d); setStorageLoading(false) })
      .catch(err => { notify(`Error: ${err}`); setStorageLoading(false) })
  }

  // ─── Batch rename ─────────────────────────────────────────────────────────────

  function openBatchRename() {
    if (selected.length < 2) { notify('Select 2 or more files to batch rename'); return }
    const items = selected.map(p => ({ from: p, name: basename(p), to: basename(p) }))
    setRenameItems(items)
    setRenamePrefix(''); setRenameSuffix(''); setRenameFindVal(''); setRenameReplaceVal(''); setRenameNumberStart('')
    setShowRenameModal(true)
  }

  const renamePreview = useMemo(() => {
    return renameItems.map((item, idx) => {
      const ext = item.name.includes('.') && !item.name.startsWith('.')
        ? item.name.slice(item.name.lastIndexOf('.'))
        : ''
      const stem = ext ? item.name.slice(0, item.name.lastIndexOf('.')) : item.name
      let newStem = stem
      if (renameFindVal) newStem = newStem.split(renameFindVal).join(renameReplaceVal)
      if (renamePrefix)  newStem = renamePrefix + newStem
      if (renameSuffix)  newStem = newStem + renameSuffix
      if (renameNumberStart !== '') {
        const n = parseInt(renameNumberStart, 10) + idx
        newStem = newStem + (isNaN(n) ? '' : `_${String(n).padStart(3, '0')}`)
      }
      return newStem + ext
    })
  }, [renameItems, renamePrefix, renameSuffix, renameFindVal, renameReplaceVal, renameNumberStart])

  function applyBatchRename() {
    const payload = renameItems.map((item, idx) => ({ from: item.from, to_name: renamePreview[idx] }))
    invoke<number>('batch_rename', { items: payload })
      .then(n => {
        notify(`Renamed ${n} file(s)`)
        setShowRenameModal(false); setSelected([]); refresh()
      })
      .catch(err => notify(`Rename failed: ${err}`))
  }

  // ─── Tags ─────────────────────────────────────────────────────────────────────

  function setTag(path: string, tag: string | null) {
    invoke('set_file_tag', { path, tag })
      .then(() => {
        setFileTags(prev => {
          const next = { ...prev }
          if (tag) next[path] = tag; else delete next[path]
          return next
        })
        notify(tag ? `Tagged as ${tag}` : 'Tag removed')
      })
      .catch(err => notify(`Tag error: ${err}`))
  }

  // ─── Command palette actions ───────────────────────────────────────────────────

  const paletteActions = useCallback((): PaletteAction[] => [
    { id: 'organize', label: 'Organize Folder', description: 'Run Tally sweep on current folder', run: () => { setShowPalette(false); openSweepPreview() } },
    { id: 'undo', label: 'Undo Last Sweep', description: 'Revert the most recent organization', run: () => { setShowPalette(false); handleUndo(1) } },
    { id: 'history', label: 'View History', description: 'Browse organization sessions', run: () => { setShowPalette(false); openHistory() } },
    { id: 'dupes', label: 'Find Duplicates', description: 'Scan for duplicate files in this folder', run: () => { setShowPalette(false); openDupeScanner() } },
    { id: 'storage', label: 'Storage Breakdown', description: 'See what\'s taking up space', run: () => { setShowPalette(false); openStorage() } },
    { id: 'rename', label: 'Batch Rename', description: 'Rename multiple selected files at once', run: () => { setShowPalette(false); openBatchRename() } },
    { id: 'newTab', label: 'New Tab', description: 'Open a new explorer tab', run: () => { setShowPalette(false); openNewTab() } },
    { id: 'newFolder', label: 'New Folder', description: 'Create a new folder here', run: () => { setShowPalette(false); setNewFolderName(''); setShowNewFolderModal(true) } },
    { id: 'watch', label: watchedFolders.includes(currentPath ?? '') ? 'Stop Watching Folder' : 'Watch Folder', description: 'Toggle background auto-clean', run: () => { setShowPalette(false); toggleWatchFolder() } },
    { id: 'settings', label: 'Settings', description: 'Open Tally config and routing rules', run: () => { setShowPalette(false); openConfig() } },
    { id: 'theme', label: dark ? 'Switch to Light Theme' : 'Switch to Dark Theme', description: 'Toggle colour theme', run: () => { setShowPalette(false); setDark(d => !d) } },
    { id: 'refresh', label: 'Refresh', description: 'Reload current folder', run: () => { setShowPalette(false); refresh() } },
    { id: 'help', label: 'Help & Shortcuts', description: 'Learn what Tally does and how to use it', run: () => { setShowPalette(false); setShowHelp(true) } },
    ...specialLocations.slice(0, 8).map(loc => ({
      id: `nav-${loc.name}`,
      label: `Go to ${loc.name}`,
      description: loc.path,
      run: () => { setShowPalette(false); navigateTo(loc.path, loc.name) },
    })),
  ], [watchedFolders, currentPath, dark, specialLocations])

  const filteredPaletteActions = useMemo(() => {
    const q = paletteQuery.trim().toLowerCase()
    if (!q) return paletteActions()
    return paletteActions().filter(a =>
      a.label.toLowerCase().includes(q) || (a.description ?? '').toLowerCase().includes(q)
    )
  }, [paletteQuery, paletteActions])

  // ─── Derived file list ────────────────────────────────────────────────────────

  const allFiles = useMemo((): FileItem[] =>
    (directory?.entries ?? []).map(e => ({
      name: e.name,
      kind: kindFromEntry(e),
      modified: fmtModified(e.modified),
      size: fmtBytes(e.size, e.is_dir),
      path: e.path,
      isDir: e.is_dir,
      bytes: e.size,
      modifiedTimestamp: e.modified,
    })),
  [directory])

  const categoryCounts = useMemo(() => ({
    all:       allFiles.length,
    folders:   allFiles.filter(f => f.isDir).length,
    documents: allFiles.filter(f => f.kind === 'document').length,
    images:    allFiles.filter(f => f.kind === 'image').length,
    videos:    allFiles.filter(f => f.kind === 'video').length,
    audio:     allFiles.filter(f => f.kind === 'audio').length,
    code:      allFiles.filter(f => f.kind === 'code').length,
    archives:  allFiles.filter(f => f.kind === 'archive').length,
  }), [allFiles])

  const filteredFiles = useMemo(() => {
    let r = [...allFiles]
    if (categoryFilter !== 'all') {
      if (categoryFilter === 'folders') r = r.filter(f => f.isDir)
      else r = r.filter(f => !f.isDir && f.kind === categoryFilter)
    }
    if (query.trim()) {
      const q = query.toLowerCase()
      r = r.filter(f => f.name.toLowerCase().includes(q))
    }
    r.sort((a, b) => {
      if (a.isDir && !b.isDir) return -1
      if (!a.isDir && b.isDir) return 1
      let c = 0
      if (sortBy === 'name')     c = a.name.localeCompare(b.name, undefined, { sensitivity: 'base' })
      else if (sortBy === 'modified') c = (b.modifiedTimestamp ?? 0) - (a.modifiedTimestamp ?? 0)
      else if (sortBy === 'size')     c = (b.bytes ?? 0) - (a.bytes ?? 0)
      else if (sortBy === 'type')     c = a.kind.localeCompare(b.kind)
      return sortAsc ? c : -c
    })
    return r
  }, [allFiles, categoryFilter, query, sortBy, sortAsc])

  const breadcrumbs = useMemo(() => {
    if (!currentPath) return []
    const parts = currentPath.split(/[/\\]/).filter(Boolean)
    let acc = ''
    return parts.map((part, i) => {
      acc += (i === 0 && currentPath.includes(':') ? `${part}\\` : `\\${part}`)
      return { label: part, path: acc }
    })
  }, [currentPath])

  const selectedItem = selected.length === 1 ? allFiles.find(f => f.path === selected[0]) ?? null : null
  const isWatched = currentPath ? watchedFolders.includes(currentPath) : false

  // ─── Render ───────────────────────────────────────────────────────────────────

  return (
    <div
      className={`tally-app ${dark ? 'theme-dark' : 'theme-light'}`}
      onDragOver={e => { e.preventDefault(); setIsDraggingOver(true) }}
      onDragLeave={e => { if (e.clientX <= 0 || e.clientY <= 0) setIsDraggingOver(false) }}
      onDrop={e => { e.preventDefault(); setIsDraggingOver(false); openSweepPreview() }}
    >
      {/* ── Title bar & tabs ──────────────────────────────────────────────────── */}
      <header className="titlebar">
        <div className="tab-strip">
          <div className="tabs">
            {tabs.map((tab, idx) => (
              <button
                key={idx}
                className={`tab ${activeTab === idx ? 'active' : ''}`}
                onClick={() => setActiveTab(idx)}
              >
                <FolderOpen size={13} className="tab-icon" />
                <span className="tab-title">{tab.name}</span>
                {tabs.length > 1 && (
                  <span className="tab-close" onClick={e => closeTab(idx, e)}><X size={11} /></span>
                )}
              </button>
            ))}
            <button className="new-tab icon-button" onClick={() => openNewTab()} title="New Tab (Ctrl+T)">
              <Plus size={14} />
            </button>
          </div>
        </div>

        <div className="titlebar-actions">
          <button className="icon-button" onClick={() => { setPaletteQuery(''); setShowPalette(true) }} title="Command Palette (Ctrl+K)">
            <Zap size={14} />
          </button>
          <button className="icon-button" onClick={() => setDark(!dark)} title="Toggle theme">
            {dark ? <Sun size={14} /> : <Moon size={14} />}
          </button>
          <button className="icon-button" onClick={openConfig} title="Settings">
            <Settings size={14} />
          </button>
        </div>
      </header>

      {/* ── Navigation bar ────────────────────────────────────────────────────── */}
      <div className="navigation-bar">
        <div className="nav-controls">
          <button className="icon-button" onClick={goBack}    disabled={!currentTab || currentTab.historyIndex <= 0}                           title="Back (Alt+Left)"><ArrowLeft  size={15} /></button>
          <button className="icon-button" onClick={goForward} disabled={!currentTab || currentTab.historyIndex >= currentTab.history.length - 1} title="Forward (Alt+Right)"><ArrowRight size={15} /></button>
          <button className="icon-button" onClick={goUp}      disabled={!directory?.parent}                                                      title="Up (Alt+Up)"><ArrowUp size={15} /></button>
          <button className="icon-button" onClick={refresh}                                                                                       title="Refresh (F5)"><RefreshCw size={14} /></button>
        </div>

        <div className="address-bar-wrapper">
          {editingPath ? (
            <input
              ref={pathInputRef}
              className="path-text-input"
              value={pathInput}
              onChange={e => setPathInput(e.target.value)}
              onKeyDown={e => {
                if (e.key === 'Enter') { navigateTo(pathInput); setEditingPath(false) }
                else if (e.key === 'Escape') setEditingPath(false)
              }}
              onBlur={() => setEditingPath(false)}
              autoFocus
            />
          ) : (
            <div className="address-breadcrumbs" onClick={() => { setPathInput(currentPath || ''); setEditingPath(true) }}>
              <Home size={13} className="address-home" onClick={e => { e.stopPropagation(); navigateTo(specialLocations[0]?.path || '') }} />
              {breadcrumbs.map((c, i) => (
                <span key={i} className="crumb-segment">
                  <ChevronRight size={11} className="crumb-arrow" />
                  <span className="crumb-label" onClick={e => { e.stopPropagation(); navigateTo(c.path) }}>{c.label}</span>
                </span>
              ))}
            </div>
          )}
        </div>

        <div className="search-box">
          <Search size={14} />
          <input
            placeholder={`Search ${currentTab?.name ?? 'folder'}`}
            value={query}
            onChange={e => setQuery(e.target.value)}
          />
          {query && <button className="clear-search" onClick={() => setQuery('')}><X size={12} /></button>}
        </div>
      </div>

      {/* ── Command toolbar ────────────────────────────────────────────────────── */}
      <div className="command-bar">
        <div className="command-group">
          <button className="command-btn tally-primary-btn" onClick={openSweepPreview} title="Organize folder with Tally">
            <Sparkles size={14} /><span>Organize</span>
          </button>
          <button className="command-btn" onClick={() => handleUndo(1)} title="Undo last sweep">
            <Undo2 size={14} /><span>Undo</span>
          </button>
          <button className="command-btn" onClick={openHistory} title="Organization history">
            <History size={14} /><span>History</span>
          </button>
          <button className={`command-btn ${isWatched ? 'watch-active' : ''}`} onClick={toggleWatchFolder} title={isWatched ? 'Stop watching' : 'Watch folder'}>
            <Radio size={13} className={isWatched ? 'pulsing-icon' : ''} /><span>{isWatched ? 'Watching' : 'Watch'}</span>
          </button>

          <span className="command-divider" />

          <button className="command-btn" onClick={openDupeScanner} title="Find duplicate files">
            <Copy size={14} /><span>Find Dupes</span>
          </button>
          <button className="command-btn" onClick={openStorage} title="Storage breakdown">
            <LayoutGrid size={14} /><span>Storage</span>
          </button>
          {selected.length >= 2 && (
            <button className="command-btn" onClick={openBatchRename} title="Batch rename selected files">
              <FileText size={14} /><span>Rename</span>
            </button>
          )}

          <span className="command-divider" />

          <button className="command-btn" onClick={() => { setShowNewFolderModal(true); setNewFolderName('') }} title="New folder">
            <FolderPlus size={14} /><span>New Folder</span>
          </button>
          {selected.length > 0 && (
            <button className="command-btn danger" onClick={handleDeleteSelected} title="Delete selected">
              <Trash2 size={14} /><span>Delete</span>
            </button>
          )}
        </div>

        <div className="command-group right">
          <div className="sort-cycle">
            {(['name','modified','size','type'] as const).map(col => (
              <button
                key={col}
                className={`command-btn ${sortBy === col ? 'sort-active' : ''}`}
                onClick={() => { if (sortBy === col) setSortAsc(!sortAsc); else { setSortBy(col); setSortAsc(true) } }}
                title={`Sort by ${col}`}
              >
                <SlidersHorizontal size={12} />
                <span>{col}{sortBy === col ? (sortAsc ? ' ▲' : ' ▼') : ''}</span>
              </button>
            ))}
          </div>
          <div className="view-switch">
            <button className={`icon-button ${view === 'list' ? 'active' : ''}`} onClick={() => setView('list')} title="List view"><List size={14} /></button>
            <button className={`icon-button ${view === 'grid' ? 'active' : ''}`} onClick={() => setView('grid')} title="Grid view"><LayoutGrid size={14} /></button>
          </div>
          <button className={`icon-button ${showDetails ? 'active' : ''}`} onClick={() => setShowDetails(!showDetails)} title="Toggle details panel">
            <PanelLeftClose size={14} />
          </button>
        </div>
      </div>

      {/* ── Category chips ─────────────────────────────────────────────────────── */}
      <div className="category-chips-bar">
        {[
          { key: 'all',      label: 'All',       count: categoryCounts.all },
          { key: 'folders',  label: 'Folders',   count: categoryCounts.folders },
          { key: 'document', label: 'Documents', count: categoryCounts.documents },
          { key: 'image',    label: 'Images',    count: categoryCounts.images },
          { key: 'video',    label: 'Videos',    count: categoryCounts.videos },
          { key: 'code',     label: 'Code',      count: categoryCounts.code },
          { key: 'archive',  label: 'Archives',  count: categoryCounts.archives },
        ].filter(c => c.key === 'all' || c.count > 0).map(cat => (
          <button key={cat.key} className={`category-chip ${categoryFilter === cat.key ? 'active' : ''}`} onClick={() => setCategoryFilter(cat.key)}>
            <span>{cat.label}</span><span className="chip-count">{cat.count}</span>
          </button>
        ))}
      </div>

      {/* ── Main workspace ─────────────────────────────────────────────────────── */}
      <div className="workspace">
        {/* Sidebar */}
        <aside className="sidebar">
          <div className="sidebar-section">
            <div className="sidebar-heading">QUICK ACCESS</div>
            {specialLocations.filter(l => !l.is_drive).map(loc => {
              const icons: Record<string, ReactNode> = {
                Home:      <Home size={15} />, Desktop: <Monitor size={15} />,
                Downloads: <ArrowDown size={15} />, Documents: <FileText size={15} />,
                Pictures:  <Image size={15} />, Music: <Music2 size={15} />, Videos: <Film size={15} />,
              }
              return (
                <button
                  key={loc.name}
                  className={`sidebar-item ${currentPath === loc.path ? 'selected' : ''}`}
                  onClick={() => navigateTo(loc.path, loc.name)}
                >
                  {icons[loc.name] ?? <Folder size={15} />}
                  <span>{loc.name}</span>
                  {watchedFolders.includes(loc.path) && <span className="watched-badge" title="Auto-clean active">•</span>}
                </button>
              )
            })}
          </div>
          <div className="sidebar-section">
            <div className="sidebar-heading">DRIVES</div>
            {specialLocations.filter(l => l.is_drive).map(loc => (
              <button
                key={loc.name}
                className={`sidebar-item ${currentPath === loc.path ? 'selected' : ''}`}
                onClick={() => navigateTo(loc.path, loc.name)}
              >
                <HardDrive size={15} /><span>{loc.name}</span>
              </button>
            ))}
          </div>
        </aside>

        {/* File area */}
        <main className="file-area">
          {loading && <div className="explorer-state"><span>Loading…</span></div>}
          {loadError && !loading && (
            <div className="explorer-state error">
              <AlertCircle size={28} />
              <strong>Could not open folder</strong>
              <p>{loadError}</p>
              <button className="primary-button" onClick={refresh}>Retry</button>
            </div>
          )}
          {!loading && !loadError && (
            <div className={`file-container ${view === 'grid' ? 'grid-mode' : 'list-mode'}`} onClick={() => setSelected([])}>
              {view === 'list' && (
                <div className="list-header">
                  <span className="col-name"     onClick={() => { setSortBy('name');     setSortAsc(!sortAsc) }}>Name {sortBy === 'name'     && (sortAsc ? '▲' : '▼')}</span>
                  <span className="col-modified"  onClick={() => { setSortBy('modified'); setSortAsc(!sortAsc) }}>Modified {sortBy === 'modified' && (sortAsc ? '▲' : '▼')}</span>
                  <span className="col-size"      onClick={() => { setSortBy('size');     setSortAsc(!sortAsc) }}>Size {sortBy === 'size'     && (sortAsc ? '▲' : '▼')}</span>
                </div>
              )}
              <div className="file-items">
                {filteredFiles.map(item => {
                  const tagColour = fileTags[item.path] ? TAG_COLOURS[fileTags[item.path]] : null
                  return (
                    <div
                      key={item.path}
                      className={`file-row ${selected.includes(item.path) ? 'selected' : ''}`}
                      onClick={e => { e.stopPropagation(); toggleSelect(item.path, e) }}
                      onDoubleClick={() => handleDoubleClick(item)}
                      onContextMenu={e => handleContextMenu(e, item)}
                    >
                      <div className="file-name-block">
                        {tagColour && <span className="tag-dot" style={{ background: tagColour }} />}
                        <FileGlyph item={item} large={view === 'grid'} />
                        <span className="file-title" title={item.name}>{item.name}</span>
                      </div>
                      {view === 'list' && (
                        <>
                          <span className="col-modified">{item.modified}</span>
                          <span className="col-size">{item.size}</span>
                        </>
                      )}
                    </div>
                  )
                })}
                {filteredFiles.length === 0 && (
                  <div className="empty-folder-message">
                    <Folder size={38} strokeWidth={1.3} />
                    <span>No matching files</span>
                  </div>
                )}
              </div>
            </div>
          )}
        </main>

        {/* Details panel */}
        {showDetails && (
          <aside className="details-sidebar">
            {selectedItem ? (
              <div className="details-body">
                <div className="preview-card">
                  {previewLoading && <div className="preview-loading">Loading preview…</div>}
                  {!previewLoading && previewData?.kind === 'image' && (
                    <div className="image-preview-wrapper">
                      <img src={previewData.content} alt={selectedItem.name} className="live-image-preview" />
                    </div>
                  )}
                  {!previewLoading && previewData?.kind === 'text' && (
                    <div className="text-preview-wrapper">
                      <div className="preview-header">
                        <Eye size={11} /><span>PREVIEW {previewData.line_count ? `(${previewData.line_count} lines)` : ''}</span>
                      </div>
                      <pre className="live-text-preview">{previewData.content}</pre>
                    </div>
                  )}
                  {!previewLoading && (!previewData || previewData.kind === 'binary') && (
                    <div className="details-icon-container"><FileGlyph item={selectedItem} large /></div>
                  )}
                </div>

                <h3 className="details-name" title={selectedItem.name}>{selectedItem.name}</h3>
                <span className="details-kind">{selectedItem.kind.toUpperCase()}</span>

                <div className="details-meta-list">
                  <div className="meta-row"><span className="meta-label">Path:</span><span className="meta-value path-value" title={selectedItem.path}>{selectedItem.path}</span></div>
                  <div className="meta-row"><span className="meta-label">Size:</span><span className="meta-value">{selectedItem.size}</span></div>
                  <div className="meta-row"><span className="meta-label">Modified:</span><span className="meta-value">{selectedItem.modified}</span></div>
                </div>

                {/* Tag picker */}
                <div className="tag-picker">
                  <div className="tag-picker-label"><Tag size={11} /> Tag</div>
                  <div className="tag-swatches">
                    {Object.entries(TAG_COLOURS).map(([name, colour]) => {
                      const active = fileTags[selectedItem.path] === name
                      return (
                        <button
                          key={name}
                          className={`tag-swatch ${active ? 'active' : ''}`}
                          style={{ background: colour }}
                          title={name}
                          onClick={() => setTag(selectedItem.path, active ? null : name)}
                        />
                      )
                    })}
                    {fileTags[selectedItem.path] && (
                      <button className="tag-clear" onClick={() => setTag(selectedItem.path, null)} title="Remove tag"><X size={10} /></button>
                    )}
                  </div>
                </div>

                <div className="details-actions">
                  <button className="secondary-button" onClick={() => invoke('open_path', { path: selectedItem.path })}>
                    <ExternalLink size={13} /> Open
                  </button>
                  <button className="secondary-button" onClick={() => invoke('reveal_in_file_manager', { path: selectedItem.path })}>
                    <Folder size={13} /> Reveal
                  </button>
                </div>
              </div>
            ) : (
              <div className="details-empty">
                <FolderOpen size={34} strokeWidth={1.3} />
                <span>Select a file to preview</span>
              </div>
            )}
          </aside>
        )}
      </div>

      {/* ── Status bar ─────────────────────────────────────────────────────────── */}
      <footer className="statusbar">
        <span className="status-item">{filteredFiles.length} item{filteredFiles.length !== 1 ? 's' : ''}</span>
        {selected.length > 0 && <span className="status-item">{selected.length} selected</span>}
        <span className="status-spacer" />
        <span className="status-item">{isWatched ? '⚡ Auto-clean active' : 'Tally v1.0.0'}</span>
      </footer>

      {/* ── Drag & drop overlay ────────────────────────────────────────────────── */}
      {isDraggingOver && (
        <div className="dropzone-overlay">
          <div className="dropzone-box">
            <Sparkles size={44} className="dropzone-icon" />
            <h2>Drop to Organize</h2>
            <p>Tally will sort these files safely into configured destinations.</p>
          </div>
        </div>
      )}

      {/* ── Context menu ───────────────────────────────────────────────────────── */}
      {contextMenu && (
        <div className="context-menu" style={{ top: contextMenu.y, left: contextMenu.x }} onClick={e => e.stopPropagation()}>
          <button onClick={() => { handleDoubleClick(contextMenu.item); setContextMenu(null) }}><ExternalLink size={13} /> Open</button>
          <button onClick={() => { invoke('reveal_in_file_manager', { path: contextMenu.item.path }); setContextMenu(null) }}><Folder size={13} /> Reveal in Explorer</button>
          <button onClick={() => { navigator.clipboard.writeText(contextMenu.item.path); notify('Copied path'); setContextMenu(null) }}><Copy size={13} /> Copy Path</button>
          {contextMenu.item.kind === 'archive' && <button onClick={() => { extractArchive(contextMenu.item); setContextMenu(null) }}><FileArchive size={13} /> Extract safely</button>}
          {!contextMenu.item.isDir && contextMenu.item.kind === 'other' && <button onClick={() => { teachTally(contextMenu.item); setContextMenu(null) }}><Sparkles size={13} /> Teach Tally this type</button>}
          <div className="context-menu-divider" />
          <div className="context-menu-submenu-label"><Tag size={12} /> Tag as…</div>
          <div className="context-tag-row">
            {Object.entries(TAG_COLOURS).map(([name, colour]) => (
              <button
                key={name}
                className="context-tag-swatch"
                style={{ background: colour }}
                title={name}
                onClick={() => { setTag(contextMenu.item.path, fileTags[contextMenu.item.path] === name ? null : name); setContextMenu(null) }}
              />
            ))}
          </div>
          <div className="context-menu-divider" />
          <button className="danger" onClick={() => { handleDeleteSelected(); setContextMenu(null) }}><Trash2 size={13} /> Delete</button>
        </div>
      )}

      {/* ── Command Palette ─────────────────────────────────────────────────────── */}
      {routeItem && (
        <div className="modal-backdrop" onClick={() => setRouteItem(null)}>
          <div className="modal-container small-modal" onClick={e => e.stopPropagation()}>
            <div className="modal-header"><h2>Teach Tally this file type</h2><button className="icon-button" onClick={() => setRouteItem(null)}><X size={15} /></button></div>
            <div className="modal-body">
              <p className="modal-subtitle"><strong>.{routeItem.name.split('.').pop()}</strong> is not in your routing rules yet. Choose its home once and Tally remembers it.</p>
              <label className="rename-row"><span>Folder</span><select className="modal-text-input" value={routeDestination} onChange={e => setRouteDestination(e.target.value)}><option value="{documents}/Other">Documents / Other</option><option value="{documents}">Documents</option><option value="{pictures}">Pictures</option><option value="{videos}">Videos</option><option value="{music}">Music</option><option value="{documents}/Data">Documents / Data</option><option value="{documents}/Code">Documents / Code</option><option value="{documents}/Archives">Documents / Archives</option></select></label>
              <label className="rename-row"><span>Rule name</span><input className="modal-text-input" value={routeLabel} onChange={e => setRouteLabel(e.target.value)} /></label>
            </div>
            <div className="modal-footer"><button className="secondary-button" onClick={() => setRouteItem(null)}>Cancel</button><button className="primary-button" onClick={saveFileType}>Save rule</button></div>
          </div>
        </div>
      )}

      {showHelp && (
        <div className="modal-backdrop" onClick={() => setShowHelp(false)}>
          <div className="modal-container" onClick={e => e.stopPropagation()}>
            <div className="modal-header"><div className="modal-header-title"><CircleHelp size={17} /><h2>Welcome to Tally</h2></div><button className="icon-button" onClick={() => setShowHelp(false)}><X size={15} /></button></div>
            <div className="modal-body">
              <p className="modal-subtitle">Tally keeps Downloads and Desktop under control. It previews every organization action and records moves so you can undo them.</p>
              <div className="settings-section"><h4>Start here</h4><p className="setting-desc">Open a folder, select <strong>Organize</strong>, review the plan, then confirm. Use <strong>Teach Tally this type</strong> on an unfamiliar file to save where it belongs.</p></div>
              <div className="settings-section"><h4>Shortcuts</h4><div className="rules-table"><div className="rule-row"><strong>Ctrl + K</strong><span className="rule-dest">Command Palette</span></div><div className="rule-row"><strong>Ctrl + /</strong><span className="rule-dest">Help &amp; Shortcuts</span></div><div className="rule-row"><strong>Esc</strong><span className="rule-dest">Close menus and dialogs</span></div><div className="rule-row"><strong>Ctrl + click</strong><span className="rule-dest">Add or remove selection</span></div><div className="rule-row"><strong>Shift + click</strong><span className="rule-dest">Select a file range</span></div></div></div>
              <div className="settings-section"><h4>Safety promise</h4><p className="setting-desc">Tally does not overwrite or delete files when it organizes. Clone moves and batch renames are available in History for rollback.</p></div>
            </div>
            <div className="modal-footer"><button className="primary-button" onClick={() => setShowHelp(false)}>Got it</button></div>
          </div>
        </div>
      )}

      {showPalette && (
        <div className="modal-backdrop palette-backdrop" onClick={() => setShowPalette(false)}>
          <div className="palette-container" onClick={e => e.stopPropagation()}>
            <div className="palette-search-row">
              <Search size={15} className="palette-search-icon" />
              <input
                ref={paletteInputRef}
                className="palette-input"
                placeholder="Search commands, locations…"
                value={paletteQuery}
                onChange={e => setPaletteQuery(e.target.value)}
                onKeyDown={e => {
                  if (e.key === 'Enter' && filteredPaletteActions[0]) filteredPaletteActions[0].run()
                  if (e.key === 'Escape') setShowPalette(false)
                }}
              />
              <kbd className="palette-esc">esc</kbd>
            </div>
            <div className="palette-list">
              {filteredPaletteActions.length === 0 && (
                <div className="palette-empty">No matching commands</div>
              )}
              {filteredPaletteActions.map(action => (
                <button key={action.id} className="palette-item" onClick={action.run}>
                  <span className="palette-label">{action.label}</span>
                  {action.description && <span className="palette-desc">{action.description}</span>}
                </button>
              ))}
            </div>
          </div>
        </div>
      )}

      {/* ── Sweep panel ────────────────────────────────────────────────────────── */}
      {showSweepPanel && (
        <div className="modal-backdrop" onClick={() => setShowSweepPanel(false)}>
          <div className="modal-container sweep-modal" onClick={e => e.stopPropagation()}>
            <div className="modal-header">
              <div className="modal-header-title"><Sparkles size={17} className="sparkle-icon" /><h2>Organize Folder</h2></div>
              <button className="icon-button" onClick={() => setShowSweepPanel(false)}><X size={15} /></button>
            </div>
            <div className="modal-body">
              <p className="modal-subtitle">Tally will sort top-level files in <strong>{currentTab?.name}</strong> by type and date. Sub-folders are never moved. Every action is undoable.</p>
              {sweepLoading && <div className="loading-spinner">Analysing…</div>}
              {sweepPlan?.warning && <div className="warning-banner"><AlertCircle size={16} /><span>{sweepPlan.warning}</span></div>}
              {sweepPlan && !sweepPlan.warning && !sweepResult && (
                <>
                  <div className="sweep-stats-grid">
                    <div className="stat-card"><span className="stat-number green">{sweepPlan.summary.moved}</span><span className="stat-label">To Move</span></div>
                    <div className="stat-card"><span className="stat-number amber">{sweepPlan.summary.duplicates}</span><span className="stat-label">Duplicates</span></div>
                    <div className="stat-card"><span className="stat-number muted">{sweepPlan.summary.skipped}</span><span className="stat-label">Skipped</span></div>
                  </div>
                  <div className="plan-list-wrapper">
                    <h4>Planned operations</h4>
                    <div className="plan-list">
                      {sweepPlan.actions.filter(a => a.kind === 'Move' || a.kind === 'Duplicate').map((a, i) => (
                        <div key={i} className="plan-row">
                          <span className={`plan-badge ${a.kind.toLowerCase()}`}>{a.kind === 'Duplicate' ? 'COPY' : 'MOVE'}</span>
                          <div className="plan-names">
                            <strong>{basename(a.source)}</strong>
                            <span className="plan-dest">→ {a.destination ? basename(a.destination) : ''}</span>
                          </div>
                          <span className="plan-rule">{a.rule_name || a.reason}</span>
                        </div>
                      ))}
                      {sweepPlan.summary.moved + sweepPlan.summary.duplicates === 0 && (
                        <div className="no-plan-actions">No loose files eligible for sorting.</div>
                      )}
                    </div>
                  </div>
                </>
              )}
              {sweepResult && (
                <div className="sweep-success-view">
                  <div className="success-icon"><Check size={28} /></div>
                  <h3>Done</h3>
                  <p>{sweepResult.moved} moved · {sweepResult.duplicates} duplicates preserved</p>
                  <div className="success-actions">
                    <button className="secondary-button" onClick={() => { handleUndo(1); setShowSweepPanel(false) }}><Undo2 size={14} /> Undo</button>
                    <button className="primary-button"   onClick={() => setShowSweepPanel(false)}>Close</button>
                  </div>
                </div>
              )}
            </div>
            <div className="modal-footer">
              <button className="secondary-button" onClick={() => setShowSweepPanel(false)}>Cancel</button>
              {sweepPlan && !sweepResult && (
                <button className="primary-button" disabled={sweepPlan.summary.moved + sweepPlan.summary.duplicates === 0 || sweepLoading} onClick={runSweep}>
                  <Sparkles size={14} /> Apply
                </button>
              )}
            </div>
          </div>
        </div>
      )}

      {/* ── Duplicate scanner ──────────────────────────────────────────────────── */}
      {showDupeModal && (
        <div className="modal-backdrop" onClick={() => setShowDupeModal(false)}>
          <div className="modal-container dupe-modal" onClick={e => e.stopPropagation()}>
            <div className="modal-header">
              <div className="modal-header-title"><Copy size={17} /><h2>Duplicate Files</h2></div>
              <button className="icon-button" onClick={() => setShowDupeModal(false)}><X size={15} /></button>
            </div>
            <div className="modal-body">
              {dupeLoading && <div className="loading-spinner">Scanning for duplicates…</div>}
              {!dupeLoading && dupeGroups.length === 0 && (
                <div className="empty-folder-message"><Check size={32} /><span>No duplicate files found in this folder.</span></div>
              )}
              {!dupeLoading && dupeGroups.length > 0 && (
                <>
                  <p className="modal-subtitle">
                    Found <strong>{dupeGroups.length}</strong> group{dupeGroups.length !== 1 ? 's' : ''} of duplicates.
                    Select the copies you want moved to <code>_Duplicates/</code>.
                  </p>
                  <div className="dupe-list">
                    {dupeGroups.map((g, gi) => (
                      <div key={gi} className="dupe-group">
                        <div className="dupe-group-header">
                          <span className="dupe-size">{humanBytes(g.size)} each</span>
                          <span className="dupe-wasted">saves {humanBytes(g.wasted_bytes)}</span>
                        </div>
                        {g.files.map((f, fi) => (
                          <label key={fi} className={`dupe-row ${dupeSelected.has(f.path) ? 'checked' : ''}`}>
                            <input
                              type="checkbox"
                              checked={dupeSelected.has(f.path)}
                              onChange={() => toggleDupeFile(f.path)}
                            />
                            <File size={14} />
                            <span className="dupe-name" title={f.path}>{f.name}</span>
                            <span className="dupe-modified">{fmtModified(f.modified)}</span>
                          </label>
                        ))}
                      </div>
                    ))}
                  </div>
                </>
              )}
            </div>
            <div className="modal-footer">
              <button className="secondary-button" onClick={() => setShowDupeModal(false)}>Cancel</button>
              {dupeSelected.size > 0 && (
                <button className="primary-button" onClick={isolateSelectedDupes}>
                  Move {dupeSelected.size} to _Duplicates/
                </button>
              )}
            </div>
          </div>
        </div>
      )}

      {/* ── Storage breakdown ──────────────────────────────────────────────────── */}
      {showStorageModal && (
        <div className="modal-backdrop" onClick={() => setShowStorageModal(false)}>
          <div className="modal-container storage-modal" onClick={e => e.stopPropagation()}>
            <div className="modal-header">
              <div className="modal-header-title"><LayoutGrid size={17} /><h2>Storage Breakdown</h2></div>
              <button className="icon-button" onClick={() => setShowStorageModal(false)}><X size={15} /></button>
            </div>
            <div className="modal-body">
              {storageLoading && <div className="loading-spinner">Calculating…</div>}
              {!storageLoading && storageData && (
                <>
                  <div className="storage-summary-row">
                    <div className="storage-stat"><span className="storage-big">{humanBytes(storageData.total_bytes)}</span><span className="storage-label">Total</span></div>
                    <div className="storage-stat"><span className="storage-big">{storageData.file_count}</span><span className="storage-label">Files</span></div>
                    <div className="storage-stat"><span className="storage-big">{storageData.dir_count}</span><span className="storage-label">Folders</span></div>
                  </div>

                  {/* Stacked bar */}
                  {storageData.total_bytes > 0 && (
                    <div className="storage-bar-wrapper">
                      <div className="storage-bar">
                        {storageData.categories.map(cat => (
                          <div
                            key={cat.label}
                            className="storage-bar-segment"
                            style={{ width: `${(cat.total_bytes / storageData.total_bytes) * 100}%`, background: cat.color }}
                            title={`${cat.label}: ${humanBytes(cat.total_bytes)}`}
                          />
                        ))}
                      </div>
                      <div className="storage-legend">
                        {storageData.categories.map(cat => (
                          <div key={cat.label} className="legend-item">
                            <span className="legend-dot" style={{ background: cat.color }} />
                            <span>{cat.label}</span>
                            <span className="legend-val">{humanBytes(cat.total_bytes)}</span>
                          </div>
                        ))}
                      </div>
                    </div>
                  )}

                  <h4 className="storage-section-title">Largest files</h4>
                  <div className="storage-top-files">
                    {storageData.top_files.map((f, i) => (
                      <div key={i} className="storage-file-row">
                        <span className="storage-rank">#{i + 1}</span>
                        <File size={13} />
                        <span className="storage-file-name" title={f.path}>{f.name}</span>
                        <span className="storage-file-size">{humanBytes(f.size)}</span>
                      </div>
                    ))}
                  </div>
                </>
              )}
            </div>
            <div className="modal-footer">
              <button className="primary-button" onClick={() => setShowStorageModal(false)}>Close</button>
            </div>
          </div>
        </div>
      )}

      {/* ── Batch rename ────────────────────────────────────────────────────────── */}
      {showRenameModal && (
        <div className="modal-backdrop" onClick={() => setShowRenameModal(false)}>
          <div className="modal-container rename-modal" onClick={e => e.stopPropagation()}>
            <div className="modal-header">
              <div className="modal-header-title"><FileText size={17} /><h2>Batch Rename</h2></div>
              <button className="icon-button" onClick={() => setShowRenameModal(false)}><X size={15} /></button>
            </div>
            <div className="modal-body">
              <div className="rename-controls">
                <div className="rename-row">
                  <label>Prefix</label>
                  <input className="modal-text-input" placeholder="Add before name" value={renamePrefix} onChange={e => setRenamePrefix(e.target.value)} />
                </div>
                <div className="rename-row">
                  <label>Suffix</label>
                  <input className="modal-text-input" placeholder="Add after name" value={renameSuffix} onChange={e => setRenameSuffix(e.target.value)} />
                </div>
                <div className="rename-row rename-row-double">
                  <div>
                    <label>Find</label>
                    <input className="modal-text-input" placeholder="Text to find" value={renameFindVal} onChange={e => setRenameFindVal(e.target.value)} />
                  </div>
                  <div>
                    <label>Replace</label>
                    <input className="modal-text-input" placeholder="Replace with" value={renameReplaceVal} onChange={e => setRenameReplaceVal(e.target.value)} />
                  </div>
                </div>
                <div className="rename-row">
                  <label>Number from</label>
                  <input className="modal-text-input" placeholder="e.g. 1  (leave blank to skip)" value={renameNumberStart} onChange={e => setRenameNumberStart(e.target.value)} />
                </div>
              </div>
              <div className="rename-preview-list">
                <div className="rename-preview-header"><span>Original</span><span>New name</span></div>
                {renameItems.map((item, i) => (
                  <div key={i} className={`rename-preview-row ${renamePreview[i] !== item.name ? 'changed' : ''}`}>
                    <span className="rename-old" title={item.from}>{item.name}</span>
                    <span className="rename-arrow">→</span>
                    <span className="rename-new">{renamePreview[i]}</span>
                  </div>
                ))}
              </div>
            </div>
            <div className="modal-footer">
              <button className="secondary-button" onClick={() => setShowRenameModal(false)}>Cancel</button>
              <button
                className="primary-button"
                disabled={renamePreview.every((n, i) => n === renameItems[i].name)}
                onClick={applyBatchRename}
              >
                Rename {renameItems.length} files
              </button>
            </div>
          </div>
        </div>
      )}

      {/* ── History modal ───────────────────────────────────────────────────────── */}
      {showHistoryModal && (
        <div className="modal-backdrop" onClick={() => setShowHistoryModal(false)}>
          <div className="modal-container history-modal" onClick={e => e.stopPropagation()}>
            <div className="modal-header">
              <h2>History &amp; Undo</h2>
              <button className="icon-button" onClick={() => setShowHistoryModal(false)}><X size={15} /></button>
            </div>
            <div className="modal-body">
              {historyList.length === 0 ? <p className="empty-history">No recorded sessions.</p> : (
                <div className="history-session-list">
                  {historyList.map((tx, idx) => (
                    <div key={tx.transaction_id} className="history-card">
                      <div className="history-card-header">
                        <div>
                          <strong>{tx.target}</strong>
                          <span className="history-time">{new Date(tx.timestamp).toLocaleString()}</span>
                        </div>
                        <div className="history-card-actions">
                          <span className="history-count">{tx.operations.length} file{tx.operations.length !== 1 ? 's' : ''}</span>
                          {idx === 0 && (
                            <button className="history-undo-btn" onClick={() => { handleUndo(1); setShowHistoryModal(false) }}>
                              <Undo2 size={11} /> Rollback
                            </button>
                          )}
                        </div>
                      </div>
                      <div className="history-card-details">
                        {tx.operations.slice(0, 4).map((op, i) => (
                          <div key={i} className="history-op">
                            <span>{basename(op.source_path)}</span>
                            <span>→ {basename(op.destination_path)}</span>
                          </div>
                        ))}
                        {tx.operations.length > 4 && <span className="history-more">+{tx.operations.length - 4} more</span>}
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>
            <div className="modal-footer">
              <button className="secondary-button" onClick={() => handleUndo(1)}><Undo2 size={13} /> Revert last</button>
              <button className="primary-button" onClick={() => setShowHistoryModal(false)}>Close</button>
            </div>
          </div>
        </div>
      )}

      {/* ── Config modal ────────────────────────────────────────────────────────── */}
      {showConfigModal && activeConfig && (
        <div className="modal-backdrop" onClick={() => setShowConfigModal(false)}>
          <div className="modal-container config-modal" onClick={e => e.stopPropagation()}>
            <div className="modal-header">
              <h2>Settings &amp; Routing Rules</h2>
              <button className="icon-button" onClick={() => setShowConfigModal(false)}><X size={15} /></button>
            </div>
            <div className="modal-body">
              <div className="settings-section">
                <h4>Windows Explorer Integration</h4>
                <div className="setting-toggle-row">
                  <div>
                    <strong>"Organize with Tally" context menu</strong>
                    <p className="setting-desc">Right-click any folder in Explorer to quickly organize it with Tally.</p>
                  </div>
                  <button className={`toggle-switch-btn ${contextMenuEnabled ? 'active' : ''}`} onClick={toggleContextMenuIntegration}>
                    {contextMenuEnabled ? 'Enabled' : 'Disabled'}
                  </button>
                </div>
              </div>
              <div className="settings-section">
                <h4>Routing Rules</h4>
                <p className="modal-subtitle">
                  Supports date tokens: <code>&#123;YYYY&#125;</code>, <code>&#123;MM&#125;</code>, <code>&#123;month&#125;</code>, and folder tokens like <code>&#123;documents&#125;</code>.
                </p>
                <div className="rules-table">
                  {activeConfig.rules.map((rule, i) => (
                    <div key={i} className="rule-row">
                      <div className="rule-info">
                        <strong>{rule.name}</strong>
                        <span className="rule-dest">{rule.destination}</span>
                      </div>
                      <div className="rule-exts">
                        {rule.extensions.map(ext => <span key={ext} className="ext-badge">{ext}</span>)}
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            </div>
            <div className="modal-footer">
              <button className="primary-button" onClick={() => setShowConfigModal(false)}>Done</button>
            </div>
          </div>
        </div>
      )}

      {/* ── New folder modal ────────────────────────────────────────────────────── */}
      {showNewFolderModal && (
        <div className="modal-backdrop" onClick={() => setShowNewFolderModal(false)}>
          <div className="modal-container small-modal" onClick={e => e.stopPropagation()}>
            <div className="modal-header">
              <h2>New Folder</h2>
              <button className="icon-button" onClick={() => setShowNewFolderModal(false)}><X size={15} /></button>
            </div>
            <div className="modal-body">
              <input
                className="modal-text-input"
                placeholder="Folder name"
                value={newFolderName}
                onChange={e => setNewFolderName(e.target.value)}
                onKeyDown={e => e.key === 'Enter' && handleCreateFolder()}
                autoFocus
              />
            </div>
            <div className="modal-footer">
              <button className="secondary-button" onClick={() => setShowNewFolderModal(false)}>Cancel</button>
              <button className="primary-button" onClick={handleCreateFolder}>Create</button>
            </div>
          </div>
        </div>
      )}

      {/* ── Toast ──────────────────────────────────────────────────────────────── */}
      {toast && (
        <div className="toast"><Check size={13} /><span>{toast}</span></div>
      )}
    </div>
  )
}
