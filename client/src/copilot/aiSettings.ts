// Form logic for the AI copilot settings, kept free of React so it can be tested.

export const OPENAI_COMPATIBLE = 'byok_openai_compatible'

export const AI_PROVIDERS = [
  { value: 'byok_openai', label: 'OpenAI (BYOK)' },
  { value: 'byok_anthropic', label: 'Anthropic (BYOK)' },
  { value: OPENAI_COMPATIBLE, label: 'OpenAI-compatible (Ollama, LM Studio, OpenRouter…)' },
  { value: 'voltrus_ai', label: 'Voltrus AI' },
]

/** Example base URLs shown under the field. */
export const BASE_URL_EXAMPLES = [
  { name: 'Ollama', url: 'http://localhost:11434/v1' },
  { name: 'LM Studio', url: 'http://localhost:1234/v1' },
  { name: 'OpenRouter', url: 'https://openrouter.ai/api/v1' },
  { name: 'Gemini', url: 'https://generativelanguage.googleapis.com/v1beta/openai' },
]

export interface AiSettingsForm {
  provider: string
  apiKey: string
  model: string
  baseUrl: string
  clearKey: boolean
}

export interface StoredAiSettings {
  provider: string
  default_model?: string | null
  base_url?: string | null
}

export interface AiSettingsBody {
  provider: string
  api_key?: string
  default_model?: string
  base_url?: string
  clear_api_key?: boolean
}

/** Mirrors the server's check: an http(s) URL with a host, no credentials, query or fragment. */
export function baseUrlError(raw: string): string | null {
  const value = raw.trim()
  if (!value) return 'Enter the base URL'
  let url: URL
  try {
    url = new URL(value)
  } catch {
    return 'Not a valid URL'
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') return 'The URL must start with http:// or https://'
  if (!url.hostname) return 'The URL has no host'
  if (url.username || url.password) return 'Put the key in the API key field, not in the URL'
  if (url.search || url.hash) return 'Remove the query string from the URL'
  return null
}

/** What the form would send, or an error message to show instead. Empty fields keep the stored value. */
export function buildAiSettingsBody(form: AiSettingsForm, stored: StoredAiSettings): { body: AiSettingsBody } | { error: string } {
  const provider = form.provider || stored.provider
  const body: AiSettingsBody = { provider }
  const model = form.model.trim()
  const baseUrl = form.baseUrl.trim()
  if (form.apiKey) body.api_key = form.apiKey
  else if (form.clearKey) body.clear_api_key = true
  if (model) body.default_model = model

  if (provider === OPENAI_COMPATIBLE) {
    const url = baseUrl || stored.base_url || ''
    const err = baseUrlError(url)
    if (err) return { error: err }
    if (baseUrl) body.base_url = baseUrl
    if (!model && !stored.default_model) return { error: 'Enter the model name, e.g. llama3.2' }
  }
  return { body }
}

/** Switching to another provider drops the stored key on the server unless a new one is entered. */
export function keyWillBeRemoved(form: AiSettingsForm, stored: StoredAiSettings, keySet: boolean): boolean {
  if (!keySet || form.apiKey) return false
  if (form.clearKey) return true
  return !!form.provider && form.provider !== stored.provider
}
