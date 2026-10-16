'use strict';

// Interface d'ArIès : conversation en streaming (SSE), lignes d'outil, validations,
// mode privé, voix (micro → whisper, réponse lue par Piper), panneaux Historique,
// Tâches et Mémoire. Aucune ressource extérieure, aucun cookie ; seul l'identifiant de
// la conversation en cours est gardé dans le navigateur (localStorage).

const $ = (id) => document.getElementById(id);
const SVG_NS = 'http://www.w3.org/2000/svg';
const STORAGE_KEY = 'aries.conversationId';

const ui = {
  app: $('app'), topbar: $('topbar'), stage: $('stage'), log: $('log'),
  composer: $('composer'), input: $('input'), send: $('send'), mic: $('mic'),
  live: $('live'), liveText: $('live-text'), liveLevel: $('live-level'),
  avatar: $('avatar'), avatarLevel: $('avatar-level'),
  statusDot: $('status-dot'), statusText: $('status-text'), statusLive: $('status-live'),
  backdrop: $('backdrop'),
};
const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)');

// ---------- Outils DOM ----------

function h(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (value == null || value === false) continue;
    if (key === 'class') node.className = value;
    else if (key === 'text') node.textContent = value;
    else if (key.startsWith('on')) node.addEventListener(key.slice(2), value);
    else node.setAttribute(key, value === true ? '' : value);
  }
  node.append(...children.flat(Infinity).filter((child) => child != null && child !== false));
  return node;
}

function icon(name, size = 'icon-sm') {
  const svg = document.createElementNS(SVG_NS, 'svg');
  svg.setAttribute('class', `icon ${size}`.trim());
  svg.setAttribute('aria-hidden', 'true');
  const use = document.createElementNS(SVG_NS, 'use');
  use.setAttribute('href', `#i-${name}`);
  svg.append(use);
  return svg;
}

const storage = {
  get() { try { return localStorage.getItem(STORAGE_KEY); } catch { return null; } },
  set(value) { try { value ? localStorage.setItem(STORAGE_KEY, value) : localStorage.removeItem(STORAGE_KEY); } catch { /* navigation privée */ } },
};

// ---------- API ----------

class ServerError extends Error {}

async function api(path, options = {}) {
  const json = options.body != null && !(options.body instanceof Blob);
  const res = await fetch(`/api/v1${path}`, {
    ...options,
    headers: { ...(json ? { 'Content-Type': 'application/json' } : {}), ...options.headers },
  });
  if (!res.ok) {
    const body = await res.json().catch(() => ({}));
    const err = new Error(body.error?.message ?? `HTTP ${res.status}`);
    err.status = res.status;
    throw err;
  }
  return res;
}
const getJson = async (path) => (await api(path)).json();

// ---------- Dates (heure locale, français) ----------

const startOfDay = (d) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
const daysFromToday = (d) => Math.round((startOfDay(d) - startOfDay(new Date())) / 86_400_000);
const hhmm = (d) => d.toLocaleTimeString('fr-FR', { hour: '2-digit', minute: '2-digit' });
const dayMonth = (d) => `${d.getDate() === 1 ? '1er' : d.getDate()} ${d.toLocaleDateString('fr-FR', { month: 'short' })}`;
const capitalize = (s) => s.charAt(0).toUpperCase() + s.slice(1);

/** « Aujourd'hui, 08:00 », « Hier, 21:14 », « Lun. 13 oct. », « 3 oct. ». */
function pastLabel(iso) {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  const days = daysFromToday(d);
  if (days === 0) return `Aujourd'hui, ${hhmm(d)}`;
  if (days === -1) return `Hier, ${hhmm(d)}`;
  if (days > -7) return `${capitalize(d.toLocaleDateString('fr-FR', { weekday: 'short' }))} ${dayMonth(d)}`;
  return dayMonth(d);
}

/** « demain, 09:00 », « 1er nov., 10:00 ». */
function nextLabel(iso) {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso ?? '';
  const days = daysFromToday(d);
  if (days === 0) return `aujourd'hui, ${hhmm(d)}`;
  if (days === 1) return `demain, ${hhmm(d)}`;
  return `${dayMonth(d)}, ${hhmm(d)}`;
}

/** Cron (5 ou 6 champs, heure locale) → « Chaque jour », « Chaque mois »… */
function cronLabel(cron) {
  const [, , dom = '*', month = '*', dow = '*'] = cron.trim().split(/\s+/).slice(-5);
  if (dom === '*' && month === '*' && dow === '*') return 'Chaque jour';
  if (dom === '*' && month === '*') return dow === '1-5' ? 'En semaine' : 'Chaque semaine';
  if (month === '*' && dow === '*') return 'Chaque mois';
  return 'Récurrente';
}

const hostOf = (url) => { try { return new URL(url).hostname.replace(/^www\./, ''); } catch { return url ?? ''; } };

// ---------- Présence : l'état de l'avatar découle de ce qui se passe ----------

const STATES = {
  idle: { label: 'Disponible', avatar: 'au repos' },
  listening: { label: "T'écoute", avatar: 'écoute', say: "ArIès t'écoute.", dot: 'active' },
  transcribing: { label: 'Transcrit…', avatar: 'transcrit', say: 'Transcription en cours.', dot: 'active' },
  thinking: { label: 'Réfléchit…', avatar: 'réfléchit', say: 'ArIès réfléchit.', dot: 'active' },
  tool: { label: 'Cherche…', avatar: 'utilise un outil', dot: 'active' },
  speaking: { label: 'Parle', avatar: 'parle', dot: 'active' },
  awaiting: { label: 'Attend ta réponse', avatar: 'attend ta validation', say: 'ArIès attend ta validation.', dot: 'active' },
  private: { label: 'Mode privé · en local', avatar: 'mode privé' },
  error: { label: 'Souci de connexion', avatar: 'erreur', dot: 'err' },
};

const flags = { voice: null, awaiting: 0, speaking: false, tools: 0, thinking: false, error: null, private: false };
let shownState = null;

function updatePresence() {
  const state = flags.voice
    ?? (flags.awaiting > 0 ? 'awaiting'
      : flags.speaking ? 'speaking'
        : flags.tools > 0 ? 'tool'
          : flags.thinking ? 'thinking'
            : flags.error ? 'error'
              : flags.private ? 'private' : 'idle');
  ui.app.dataset.mode = flags.private ? 'private' : '';
  const label = state === 'error' ? flags.error.label : STATES[state].label;
  if (state === shownState && ui.statusText.textContent === label) return;
  shownState = state;
  const info = STATES[state];
  ui.avatar.dataset.state = state;
  ui.avatar.querySelector('svg').setAttribute('aria-label', `ArIès, ${info.avatar}`);
  ui.statusText.textContent = label;
  ui.statusDot.className = `presence-dot${info.dot ? ` is-${info.dot}` : ''}`;
  const say = state === 'error' ? flags.error.say : info.say;
  if (say) ui.statusLive.textContent = say;
}

function setPrivate(on) {
  if (on && !flags.private) ui.statusLive.textContent = 'Mode privé activé, traitement en local.';
  flags.private = on;
  updatePresence();
  updateComposer();
}

function resetActivity() {
  Object.assign(flags, { voice: null, awaiting: 0, speaking: false, tools: 0, thinking: false, error: null });
}

// ---------- Lecture à voix haute ----------

// Chaque phrase est synthétisée dès qu'elle est complète et jouée dans l'ordre ; la phrase
// lue est surlignée. Un clic sur le micro (« Couper ») interrompt tout.
const speech = {
  queue: Promise.resolve(),
  audio: null,
  generation: 0,
  pending: 0,
  get busy() { return this.pending > 0; },
  say(text, onStart, onEnd) {
    const clean = plainText(text).replace(/https?:\/\/\S+/g, 'le lien');
    if (!/[\p{L}\p{N}]/u.test(clean)) return;
    const generation = this.generation;
    this.pending++;
    updateMic();
    const wav = api('/syntheses', { method: 'POST', body: JSON.stringify({ text: clean }) })
      .then((res) => res.blob()).catch(() => null);
    this.queue = this.queue.then(async () => {
      try {
        const blob = await wav;
        if (!blob || generation !== this.generation) return;
        const url = URL.createObjectURL(blob);
        this.audio = new Audio(url);
        ui.avatar.classList.remove('is-pause');
        flags.speaking = true;
        updatePresence();
        onStart?.();
        await new Promise((done) => {
          this.audio.onended = this.audio.onerror = this.audio.onpause = done;
          this.audio.play().catch(done);
        });
        onEnd?.();
        URL.revokeObjectURL(url);
        this.audio = null;
      } finally {
        this.pending--;
        // Bouche fermée entre deux phrases.
        ui.avatar.classList.toggle('is-pause', this.pending > 0);
        if (this.pending === 0) flags.speaking = false;
        updatePresence();
        updateMic();
      }
    });
  },
  stop() {
    this.generation++;
    this.audio?.pause();
  },
};

// Morceaux d'au moins 20 caractères finissant une phrase ou une ligne ; `flush` rend le reste.
const SENTENCE = /^[\s\S]{20,}?(?:[.!?…:;]+(?=\s)|\n)/;
function sentenceSplitter(onSentence) {
  let pending = '';
  return {
    push(text) {
      pending += text;
      let match;
      while ((match = pending.match(SENTENCE))) {
        onSentence(match[0]);
        pending = pending.slice(match[0].length);
      }
    },
    flush() { if (pending) onSentence(pending); pending = ''; },
  };
}

// ---------- Fil de la conversation ----------

const externalLink = (href, ...children) => h('a', { href, target: '_blank', rel: 'noopener noreferrer' }, ...children);

// Markdown des réponses : GFM (listes, tableaux, liens nus), retours à la ligne conservés.
// Le HTML produit est nettoyé : une page lue par ArIès ne peut pas injecter de script ;
// pas d'images (elles chargeraient un site tiers), liens ouverts dans un nouvel onglet.
const markdownReady = Boolean(window.marked && window.DOMPurify);
if (markdownReady) {
  window.marked.setOptions({ gfm: true, breaks: true });
  window.DOMPurify.addHook('afterSanitizeAttributes', (node) => {
    if (node.tagName === 'A') {
      node.setAttribute('target', '_blank');
      node.setAttribute('rel', 'noopener noreferrer');
    }
  });
}

function renderMarkdown(text) {
  if (!markdownReady) return h('p', { text });
  const html = window.DOMPurify.sanitize(window.marked.parse(text), {
    FORBID_TAGS: ['img', 'style', 'form', 'input', 'button', 'iframe'],
    FORBID_ATTR: ['style'],
  });
  const template = document.createElement('template');
  template.innerHTML = html;
  return template.content;
}

/** Texte lisible d'un morceau de Markdown (lecture à voix haute, recherche du surlignage). */
const plainText = (markdown) => markdown
  .replace(/!?\[([^\]]*)\]\([^)]*\)/g, '$1')
  .replace(/^\s{0,3}(#{1,6}|>|[-*+]|\d+[.)])\s+/gm, '')
  .replace(/[*_`~|]/g, '')
  .replace(/\s+/g, ' ')
  .trim();

const BLOCKS = new Set(['P', 'UL', 'OL', 'LI', 'BLOCKQUOTE', 'H1', 'H2', 'H3', 'H4', 'H5', 'H6', 'PRE', 'TABLE', 'THEAD', 'TBODY', 'TR', 'TD', 'TH']);
const canHighlight = 'highlights' in CSS && typeof Highlight === 'function';

/** Réponse d'ArIès : Markdown, curseur pendant l'écriture, phrase lue surlignée. */
class AriesMessage {
  constructor(text = '', streaming = true) {
    this.text = text;
    this.streaming = streaming;
    this.spoken = null;
    this.body = h('div', { class: 'md' });
    this.caret = h('span', { class: 'caret', 'aria-hidden': 'true' });
    this.el = h('div', { class: 'msg msg-aries' }, this.body);
    this.render();
  }

  append(text) { this.text += text; this.render(); }
  finish() { this.streaming = false; this.render(); }

  /** Phrase en cours de lecture (Markdown brut), ou `null`. */
  setHighlight(sentence) {
    this.spoken = sentence && plainText(sentence);
    this.highlight();
  }

  render() {
    this.body.replaceChildren(renderMarkdown(this.text));
    if (this.streaming) {
      // Curseur à la fin du dernier bloc de texte (paragraphe, élément de liste…).
      let node = this.body;
      while (node.lastElementChild && BLOCKS.has(node.lastElementChild.tagName)) node = node.lastElementChild;
      node.append(this.caret);
    }
    if (this.spoken) this.highlight();
  }

  /** Surligne la phrase lue sans toucher au DOM (CSS Custom Highlight API). */
  highlight() {
    if (!canHighlight) return;
    CSS.highlights.delete('aries-speaking');
    if (!this.spoken) return;
    // Texte affiché, espaces regroupés, avec la position d'origine de chaque caractère.
    let flat = '';
    const map = [];
    const walker = document.createTreeWalker(this.body, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      for (let i = 0; i < node.data.length; i++) {
        const space = /\s/.test(node.data[i]);
        if (space && (!flat || flat.endsWith(' '))) continue;
        flat += space ? ' ' : node.data[i];
        map.push([node, i]);
      }
    }
    const start = flat.indexOf(this.spoken);
    if (start < 0) return;
    const [startNode, startOffset] = map[start];
    const [endNode, endOffset] = map[start + this.spoken.length - 1];
    const range = new Range();
    range.setStart(startNode, startOffset);
    range.setEnd(endNode, endOffset + 1);
    CSS.highlights.set('aries-speaking', new Highlight(range));
  }
}

const userBubble = (text) => h('div', { class: 'msg msg-user', text });

function sourcesBlock(urls) {
  return h('div', { class: 'sources' }, h('span', { text: 'Sources' }),
    [...urls].map((url) => externalLink(url, icon('link'), hostOf(url))));
}

// ---------- Lignes d'outil ----------

const BROWSER_ACTIONS = {
  browser_navigate_back: 'page précédente', browser_snapshot: 'lecture de la page', browser_click: 'clic',
  browser_type: 'saisie', browser_select_option: 'choix dans une liste', browser_press_key: 'touche',
  browser_wait_for: 'attente', browser_tabs: 'onglets', browser_close: 'fermeture',
};
const NOTION_ACTIONS = {
  'API-retrieve-page-markdown': 'lecture d’une page', 'API-query-data-source': 'lecture d’une base',
  'API-post-page': 'nouvelle page', 'API-patch-block-children': 'ajout de contenu',
};

/** [icône, nature, détail] d'un appel d'outil. */
function describeTool(name, a = {}) {
  switch (name) {
    case 'web_search': return ['search', 'Recherche', a.query];
    case 'read_web_page': return ['file', 'Lecture', hostOf(a.url)];
    case 'browser_navigate': return ['globe', 'Navigation', hostOf(a.url)];
    case 'search_gmail_messages': return ['mail', 'Recherche mail', a.query];
    case 'get_gmail_message_content':
    case 'get_gmail_thread_content': return ['mail', 'Lecture mail', 'contenu privé'];
    case 'draft_gmail_message': return ['mail', 'Brouillon', a.to ? `mail à ${a.to}` : 'mail'];
    case 'send_gmail_message': return ['plane', 'Envoi', a.to ? `à ${a.to}` : 'mail'];
    case 'list_calendars':
    case 'get_events': return ['calendar', 'Agenda', 'consultation'];
    case 'manage_event': return ['calendar', 'Agenda', a.summary ?? a.action];
    case 'schedule_task': {
      const when = a.at ? nextLabel(a.at) : a.cron ? cronLabel(a.cron).toLowerCase() : '';
      return ['bell', a.kind === 'agent' ? 'Tâche' : 'Rappel', [when, a.title].filter(Boolean).join(' · ')];
    }
    case 'list_tasks': return ['bell', 'Tâches', 'liste'];
    case 'cancel_task': return ['bell', 'Annulation', `tâche n° ${a.id}`];
    case 'API-post-search': return ['note', 'Notion', a.query ? `recherche : ${a.query}` : 'recherche'];
    default:
      if (name.startsWith('browser_')) return ['globe', 'Navigateur', BROWSER_ACTIONS[name] ?? name];
      if (name.startsWith('API-')) return ['note', 'Notion', NOTION_ACTIONS[name] ?? name];
      return ['tool', 'Outil', name];
  }
}

function toolLine(call) {
  const [ic, kind, detail] = describeTool(call.name, call.arguments ?? {});
  const status = h('span', { class: 'tool-status is-run' }, h('span', { class: 'run-dot' }), h('span', { class: 'sr-only', text: 'en cours' }));
  const el = h('div', { class: 'tool-line is-run' },
    h('span', { class: 'tool-ic' }, icon(ic)),
    h('span', { class: 'tool-kind', text: kind }),
    detail && h('span', { class: 'tool-detail', text: detail, title: detail }),
    status);
  return {
    el,
    finish(success, decision) {
      el.classList.remove('is-run');
      const refused = !success && decision !== 'automatic' && decision !== 'approved';
      status.className = `tool-status ${success ? 'is-ok' : refused ? 'is-refused' : 'is-fail'}`;
      status.replaceChildren(icon(success ? 'check' : 'x'),
        success ? h('span', { class: 'sr-only', text: 'terminé' }) : h('span', { text: refused ? 'refusé' : 'échec' }));
    },
  };
}

// ---------- Demande de validation (action `danger`) ----------

const EVENT_ACTIONS = { create: 'Créer', update: 'Modifier', delete: 'Supprimer' };
const dateTime = (iso) => {
  const d = new Date(iso);
  return iso && !Number.isNaN(d.getTime()) ? `${capitalize(d.toLocaleDateString('fr-FR', { weekday: 'long', day: 'numeric', month: 'long' }))}, ${hhmm(d)}` : iso;
};
const short = (value, max = 600) => {
  const text = typeof value === 'string' ? value : JSON.stringify(value);
  return text.length > max ? `${text.slice(0, max)}…` : text;
};

function approvalContent(req) {
  const a = req.arguments ?? {};
  if (req.name === 'send_gmail_message') {
    return {
      ic: 'mail', title: `Envoyer ce mail${a.to ? ` à ${a.to}` : ''} ?`, sub: "Rien ne part tant que tu n'as pas dit oui.",
      allow: "Autoriser l'envoi", fields: [['À', a.to], ['Cc', a.cc], ['Cci', a.bcc], ['Objet', a.subject]], body: a.body,
    };
  }
  if (req.name === 'manage_event') {
    return {
      ic: 'calendar', title: `${EVENT_ACTIONS[a.action] ?? 'Modifier'} un événement de ton agenda ?`,
      sub: "Rien ne change tant que tu n'as pas dit oui.", allow: 'Autoriser',
      fields: [['Titre', a.summary], ['Début', dateTime(a.start_time)], ['Fin', dateTime(a.end_time)], ['Lieu', a.location]],
      body: a.description,
    };
  }
  const [, kind, detail] = describeTool(req.name, a);
  return {
    ic: 'alert', title: `${kind}${detail ? ` : ${detail}` : ''} ?`, sub: "Rien ne se fait tant que tu n'as pas dit oui.",
    allow: 'Autoriser', fields: Object.entries(a).filter(([key]) => key !== 'user_google_email'),
  };
}

function approvalCard(req, onApproved) {
  const content = approvalContent(req);
  const titleId = `confirm-${req.approval_id}`;
  const preview = h('dl', { class: 'confirm-preview' },
    content.fields.filter(([, value]) => value != null && value !== '')
      .map(([key, value]) => [h('dt', { text: key }), h('dd', { text: short(value) })]),
    content.body && [h('dt', { class: 'sr-only', text: 'Message' }), h('dd', { class: 'body', text: short(content.body, 4000) })]);
  const allow = h('button', { class: 'btn btn-primary', type: 'button' }, icon('check'), content.allow);
  const refuse = h('button', { class: 'btn', type: 'button', text: 'Refuser' });
  const timer = h('span', { class: 'confirm-timer' });
  const actions = h('div', { class: 'confirm-actions' }, allow, refuse, timer);
  const el = h('div', { class: 'confirm', role: 'group', 'aria-labelledby': titleId },
    h('div', { class: 'confirm-head' }, icon(content.ic, ''),
      h('div', {}, h('p', { class: 'confirm-title', id: titleId, text: content.title }), h('p', { class: 'confirm-sub', text: content.sub }))),
    preview.childElementCount ? preview : null,
    actions);

  let settled = false;
  let interval = null;
  const settle = (text) => {
    if (settled) return;
    settled = true;
    clearInterval(interval);
    el.classList.add('is-done');
    actions.replaceChildren(h('p', { class: 'confirm-note', text }));
    flags.awaiting = Math.max(0, flags.awaiting - 1);
    updatePresence();
  };
  const decide = async (approved) => {
    if (settled) return;
    allow.disabled = refuse.disabled = true;
    try {
      await api(`/tool-approvals/${req.approval_id}`, { method: 'PUT', body: JSON.stringify({ approved }) });
      settle(approved ? 'Autorisé par toi.' : "Refusé : rien n'a été fait.");
      if (approved) onApproved();
    } catch (err) {
      settle(err.status === 404 ? "Trop tard : la demande avait expiré, rien n'a été fait." : `Réponse non transmise : ${err.message}`);
    }
  };
  allow.addEventListener('click', () => decide(true));
  refuse.addEventListener('click', () => decide(false));

  let left = req.expires_in_secs ?? 300;
  const tick = () => {
    timer.textContent = `Expire dans ${Math.floor(left / 60)}:${String(left % 60).padStart(2, '0')}`;
    if (left-- <= 0) settle("Délai dépassé : rien n'a été fait.");
  };
  tick();
  interval = setInterval(tick, 1000);

  return {
    el,
    focus: () => allow.focus(),
    settle: (decision) => settle(decision === 'approved' ? 'Autorisé par toi.'
      : decision === 'rejected' ? "Refusé : rien n'a été fait." : "Délai dépassé : rien n'a été fait."),
  };
}

// ---------- Encarts ----------

function privateBanner() {
  return h('div', { class: 'private-banner' }, icon('lock', ''),
    h('span', { class: 'grow' }, h('strong', { text: 'Mode privé.' }),
      " Ce contenu est personnel : la suite est traitée ici, en local. C'est plus lent."),
    h('button', { class: 'btn', type: 'button', onclick: () => newConversation(), text: 'Nouvelle conversation' }));
}

function errorCard(title, text, onRetry) {
  return h('div', { class: 'msg-error', role: 'alert' }, icon('alert', ''),
    h('span', { class: 'grow' }, h('strong', { text: title }), text ? ` ${text}` : ''),
    onRetry && h('button', { class: 'btn', type: 'button', onclick: onRetry }, icon('refresh'), 'Réessayer'));
}

// ---------- Conversation ----------

let health = {};
let voiceEnabled = false;
let conversationId = null;
let busy = false;
// Change à chaque nouvelle conversation : un flux encore ouvert n'écrit plus dans l'écran.
let session = 0;

function showWelcome() {
  const chip = (label, action) => h('button', { class: 'chip', type: 'button', onclick: action, text: label });
  ui.log.replaceChildren(h('div', { class: 'welcome' },
    h('h1', { text: 'Bonjour. Que puis-je faire pour toi ?' }),
    h('p', { text: voiceEnabled ? 'Écris-moi, ou appuie sur le micro pour me parler.' : 'Écris-moi ce dont tu as besoin.' }),
    h('div', { class: 'suggestions' },
      chip('Quel temps demain ?', () => submit('Quel temps demain ?')),
      chip('Écris un mail…', () => prefill('Écris un mail à ')),
      voiceEnabled && chip('Me parler à voix haute', () => startRecording()))));
}

function prefill(text) {
  ui.input.value = text;
  autosize();
  ui.input.focus();
  ui.input.setSelectionRange(text.length, text.length);
}

function newConversation() {
  session++;
  speech.stop();
  busy = false;
  conversationId = null;
  storage.set(null);
  resetActivity();
  flags.private = false;
  showWelcome();
  updatePresence();
  updateComposer();
  ui.input.focus();
}

async function loadConversation(id) {
  const [conversation, messages] = await Promise.all([
    getJson(`/conversations/${id}`),
    getJson(`/conversations/${id}/messages`),
  ]);
  session++;
  speech.stop();
  busy = false;
  conversationId = String(conversation.id);
  storage.set(conversationId);
  resetActivity();
  flags.private = conversation.private;
  if (messages.length) {
    ui.log.replaceChildren(...messages.map((m) => (m.role === 'user' ? userBubble(m.content) : new AriesMessage(m.content, false).el)));
  } else {
    showWelcome();
  }
  if (conversation.private) ui.log.append(privateBanner());
  updatePresence();
  updateComposer();
  return messages;
}

async function ensureConversation() {
  if (conversationId) return conversationId;
  const conversation = await (await api('/conversations', { method: 'POST' })).json();
  conversationId = String(conversation.id);
  storage.set(conversationId);
  return conversationId;
}

async function submit(text, { spoken = false } = {}) {
  text = text.trim();
  if (!text || busy) return;
  const mine = session;
  busy = true;
  flags.error = null;
  flags.thinking = true;
  updatePresence();
  updateComposer();
  try {
    await ask(text, spoken, () => mine === session);
  } catch (err) {
    if (mine === session) showFailure(err, text, spoken);
  } finally {
    if (mine === session) {
      busy = false;
      flags.thinking = false;
      flags.tools = 0;
      updatePresence();
      updateComposer();
      if (!flags.voice) ui.input.focus();
    }
  }
}

/** Envoie le message et affiche le flux SSE dans l'ordre où il arrive. */
async function ask(text, spoken, alive) {
  const id = await ensureConversation();
  ui.log.querySelector('.welcome')?.remove();
  ui.log.append(userBubble(text));

  let segment = null;
  let splitter = null;
  let lastAnswer = null;
  let finished = false;
  const calls = new Map();
  const lines = new Map();
  const cards = new Map();
  const sources = new Set();

  const closeSegment = () => {
    segment?.finish();
    splitter?.flush();
    segment = splitter = null;
  };
  const handle = (event, data) => {
    switch (event) {
      case 'token': {
        if (!segment) {
          const message = new AriesMessage();
          segment = lastAnswer = message;
          ui.log.append(message.el);
          if (spoken) {
            splitter = sentenceSplitter((sentence) =>
              speech.say(sentence, () => message.setHighlight(sentence), () => message.setHighlight(null)));
          }
        }
        segment.append(data.text);
        splitter?.push(data.text);
        flags.thinking = false;
        break;
      }
      case 'tool_call': {
        closeSegment();
        const line = toolLine(data);
        calls.set(data.call_id, data);
        lines.set(data.call_id, line);
        ui.log.append(line.el);
        flags.tools++;
        flags.thinking = false;
        break;
      }
      case 'approval_required': {
        // La carte remplace la ligne ; une fois autorisée, une nouvelle ligne suit l'exécution.
        lines.get(data.call_id)?.el.remove();
        lines.delete(data.call_id);
        flags.awaiting++;
        const card = approvalCard(data, () => {
          const line = toolLine(data);
          lines.set(data.call_id, line);
          card.el.after(line.el);
        });
        cards.set(data.call_id, card);
        ui.log.append(card.el);
        card.focus();
        break;
      }
      case 'tool_result': {
        cards.get(data.call_id)?.settle(data.decision);
        let line = lines.get(data.call_id);
        if (!line) {
          line = toolLine(calls.get(data.call_id) ?? data);
          const card = cards.get(data.call_id);
          card ? card.el.after(line.el) : ui.log.append(line.el);
        }
        line.finish(data.success, data.decision);
        flags.tools = Math.max(0, flags.tools - 1);
        const call = calls.get(data.call_id);
        if (data.success && call?.name === 'read_web_page' && call.arguments?.url) sources.add(call.arguments.url);
        break;
      }
      case 'private_mode':
        closeSegment();
        setPrivate(true);
        ui.log.append(privateBanner());
        break;
      case 'error':
        throw new ServerError(data.message);
      case 'done':
        finished = true;
        break;
      default:
    }
    updatePresence();
  };

  try {
    const res = await api(`/conversations/${id}/messages`, { method: 'POST', body: JSON.stringify({ content: text }) });
    // Flux SSE : blocs séparés par une ligne vide, champs `event:` et `data:`.
    const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
    let buffer = '';
    while (!finished) {
      const { value, done } = await reader.read();
      if (done) break;
      buffer += value;
      const blocks = buffer.split('\n\n');
      buffer = blocks.pop();
      for (const block of blocks) {
        const event = block.match(/^event: ?(.*)$/m)?.[1];
        const raw = block.match(/^data: ?(.*)$/m)?.[1];
        if (event && alive()) handle(event, raw ? JSON.parse(raw) : {});
      }
    }
    if (!finished && alive()) throw new TypeError('flux interrompu');
  } catch (err) {
    if (alive()) closeSegment();
    err.partial = lastAnswer;
    throw err;
  }
  if (!alive()) return;
  closeSegment();
  if (sources.size && lastAnswer) lastAnswer.el.append(sourcesBlock(sources));
  ui.statusLive.textContent = 'ArIès a répondu.';
}

function showFailure(err, text, spoken) {
  // Coupure réseau : le serveur continue et enregistre la réponse. Erreur serveur : rien.
  const network = !(err instanceof ServerError) && err.status == null;
  flags.error = network
    ? { label: 'Souci de connexion', say: 'Erreur : la connexion a coupé.' }
    : { label: 'Petit souci', say: `Erreur : ${err.message}` };
  if (err.partial) err.partial.el.append(h('p', { class: 'msg-interrupted', text: 'Réponse interrompue.' }));
  const card = errorCard(
    network ? 'La connexion a coupé.' : "Je n'ai pas pu répondre.",
    network ? "Désolée, je n'ai pas pu finir ma réponse. Rien n'est perdu." : err.message,
    () => retry(card, network, text, spoken));
  ui.log.append(card);
  updatePresence();
}

async function retry(card, network, text, spoken) {
  card.remove();
  flags.error = null;
  updatePresence();
  if (network && conversationId) {
    // La réponse a peut-être été enregistrée pendant la coupure : on recharge d'abord.
    try {
      const messages = await loadConversation(conversationId);
      if (messages.at(-1)?.role === 'assistant') return;
    } catch { /* on renvoie la question */ }
  }
  submit(text, { spoken });
}

// ---------- Zone de saisie et micro ----------

let recorder = null;
let meter = null;

function updateComposer() {
  const voice = flags.voice != null;
  ui.input.hidden = voice;
  ui.live.hidden = !voice;
  ui.send.disabled = busy || voice;
  ui.input.placeholder = busy ? 'ArIès répond…' : flags.private ? 'Écris à ArIès (mode privé)…' : 'Écris à ArIès…';
  updateMic();
}

function updateMic() {
  const recording = recorder != null;
  const cut = speech.busy && !recording;
  const label = recording ? 'Envoyer ce que tu as dit' : cut ? 'Couper ArIès' : 'Me parler à voix haute';
  ui.mic.classList.toggle('is-rec', recording);
  ui.mic.setAttribute('aria-pressed', String(recording));
  ui.mic.setAttribute('aria-label', label);
  ui.mic.title = label;
  ui.mic.querySelector('use').setAttribute('href', cut ? '#i-stop' : '#i-mic');
  ui.mic.disabled = flags.voice === 'transcribing' || (busy && !cut && !recording);
}

function autosize() {
  ui.input.style.height = 'auto';
  ui.input.style.height = `${Math.min(ui.input.scrollHeight, 200)}px`;
}

async function startRecording() {
  if (busy || recorder || !voiceEnabled) return;
  speech.stop();
  let stream;
  try {
    stream = await navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true } });
  } catch (err) {
    ui.log.append(errorCard('Micro indisponible.', err.name === 'NotAllowedError'
      ? "Autorise l'accès au micro pour ce site dans ton navigateur." : err.message));
    return;
  }
  const chunks = [];
  const rec = new MediaRecorder(stream);
  rec.ondataavailable = (e) => { if (e.data.size) chunks.push(e.data); };
  rec.onstop = () => transcribe(new Blob(chunks, { type: rec.mimeType || 'audio/webm' }), stream);
  recorder = rec;
  meter = startMeter(stream);
  rec.start();
  flags.voice = 'listening';
  ui.liveText.textContent = "Je t'écoute… Appuie à nouveau sur le micro pour envoyer.";
  updatePresence();
  updateComposer();
}

async function transcribe(blob, stream) {
  stream.getTracks().forEach((track) => track.stop());
  meter?.stop();
  meter = null;
  recorder = null;
  flags.voice = 'transcribing';
  ui.liveText.textContent = 'Transcription…';
  updatePresence();
  updateComposer();
  let text = '';
  try {
    const res = await api('/transcriptions', { method: 'POST', headers: { 'Content-Type': blob.type || 'audio/webm' }, body: blob });
    text = ((await res.json()).text ?? '').trim();
  } catch (err) {
    ui.log.append(errorCard("Je n'ai pas pu transcrire ce que tu as dit.", err.message));
  } finally {
    flags.voice = null;
    updatePresence();
    updateComposer();
  }
  if (text) submit(text, { spoken: true });
  else ui.statusLive.textContent = "Je n'ai rien entendu.";
}

/** Niveau du micro → hauteur des barres (sous l'avatar et dans la zone de saisie). */
function startMeter(stream) {
  const Context = window.AudioContext || window.webkitAudioContext;
  if (!Context || reducedMotion.matches) return null;
  const context = new Context();
  const analyser = context.createAnalyser();
  analyser.fftSize = 512;
  context.createMediaStreamSource(stream).connect(analyser);
  const samples = new Uint8Array(analyser.fftSize);
  const bars = [...ui.liveLevel.children, ...ui.avatarLevel.children];
  const weights = [0.55, 0.85, 1, 0.75, 0.6];
  ui.avatarLevel.classList.add('is-live');
  let frame = 0;
  const tick = () => {
    analyser.getByteTimeDomainData(samples);
    let sum = 0;
    for (const sample of samples) sum += ((sample - 128) / 128) ** 2;
    const level = Math.min(1, Math.sqrt(sum / samples.length) * 4);
    bars.forEach((bar, i) => bar.style.setProperty('--level', (0.15 + 0.85 * level * weights[i % 5]).toFixed(2)));
    frame = requestAnimationFrame(tick);
  };
  tick();
  return {
    stop() {
      cancelAnimationFrame(frame);
      context.close();
      ui.avatarLevel.classList.remove('is-live');
    },
  };
}

// ---------- Panneaux ----------

let openPanel = null;
let lastFocus = null;

async function showDrawer(name) {
  closeDrawer();
  lastFocus = document.activeElement;
  openPanel = $(`drawer-${name}`);
  ui.backdrop.hidden = false;
  openPanel.hidden = false;
  ui.topbar.inert = ui.stage.inert = true;
  openPanel.querySelector('[data-close]').focus();
  const body = openPanel.querySelector('.drawer-body');
  body.replaceChildren(h('p', { class: 'drawer-intro', text: 'Chargement…' }));
  try {
    body.replaceChildren(...(await DRAWERS[name]()).flat(Infinity).filter(Boolean));
  } catch (err) {
    body.replaceChildren(errorCard('Indisponible pour le moment.', err.message));
  }
}

function closeDrawer() {
  if (!openPanel) return;
  openPanel.hidden = true;
  ui.backdrop.hidden = true;
  ui.topbar.inert = ui.stage.inert = false;
  openPanel = null;
  lastFocus?.focus?.();
}

const empty = (ic, title, text) => h('div', { class: 'empty' }, icon(ic, ''), h('span', { text: title }), text && h('span', { text }));

/** Retire un élément de liste ; affiche `placeholder` si la liste devient vide. */
function removeItem(li, placeholder) {
  const list = li.parentElement;
  li.remove();
  if (list && !list.childElementCount) list.replaceWith(placeholder);
}

const DRAWERS = {
  async history() {
    const conversations = await getJson('/conversations');
    const none = empty('history', 'Aucune conversation pour l’instant.');
    const items = conversations.map((c) => {
      const current = String(c.id) === conversationId;
      const title = c.title || 'Nouvelle conversation';
      const li = h('li', { class: `item${current ? ' is-current' : ''}` },
        h('button', {
          class: 'item-open', type: 'button', 'aria-current': current ? 'true' : null,
          onclick: () => { closeDrawer(); loadConversation(c.id).catch((err) => ui.log.append(errorCard('Conversation indisponible.', err.message))); },
        },
        h('span', { class: 'item-title', text: title }),
        h('span', { class: 'item-meta' }, c.private && h('span', { class: 'tag', text: 'privé' }), current ? 'En cours' : pastLabel(c.updated_at))),
        h('button', {
          class: 'btn btn-ghost btn-icon', type: 'button', title: 'Effacer', 'aria-label': `Effacer « ${title} »`,
          onclick: async () => {
            if (!window.confirm('Effacer cette conversation et ses souvenirs ? C’est définitif.')) return;
            await api(`/conversations/${c.id}`, { method: 'DELETE' });
            removeItem(li, none);
            if (current) newConversation();
          },
        }, icon('trash')));
      return li;
    });
    return [
      h('button', { class: 'btn btn-primary', type: 'button', onclick: () => { closeDrawer(); newConversation(); } }, icon('plus'), 'Nouvelle conversation'),
      h('p', { class: 'section-label', text: 'Récentes' }),
      items.length ? h('ul', { class: 'list' }, items) : none,
    ];
  },

  async tasks() {
    const [tasks, notifications] = await Promise.all([getJson('/tasks'), getJson('/notifications')]);
    const none = empty('bell', 'Aucune tâche planifiée.', 'Demande-moi par exemple « rappelle-moi d’appeler le garage demain à 9 h ».');
    const items = tasks.map((task) => {
      const when = task.next_run_at ? (task.cron ? `prochain : ${nextLabel(task.next_run_at)}` : nextLabel(task.next_run_at)) : '';
      const li = h('li', { class: 'item' },
        h('div', { class: 'item-main' },
          h('span', { class: 'item-title', text: task.title }),
          h('span', { class: 'item-meta' }, h('span', { class: 'tag', text: task.cron ? cronLabel(task.cron) : 'Unique' }), when)),
        h('button', {
          class: 'btn btn-ghost', type: 'button', 'aria-label': `Annuler « ${task.title} »`, text: 'Annuler',
          onclick: async () => { await api(`/tasks/${task.id}`, { method: 'DELETE' }); removeItem(li, none); },
        }));
      return li;
    });
    const notes = notifications.map((n) => {
      const text = short(n.message.replace(/^(⏰|🔔|⚠️)\s*/u, ''), 400);
      return h('div', { class: `notif${n.success ? '' : ' is-fail'}` },
        icon(n.success ? (n.kind === 'reminder' ? 'clock' : 'check') : 'alert', ''),
        h('div', { class: 'notif-main' },
          h('span', { class: 'notif-text', text: n.kind === 'reminder' && n.success ? `Rappel : ${text}` : text }),
          h('span', { class: 'item-meta', text: `${pastLabel(n.finished_at)} · ${n.kind === 'reminder' ? 'rappel' : 'tâche de fond'}` })));
    });
    return [
      h('p', { class: 'section-label', text: 'Planifiées' }),
      items.length ? h('ul', { class: 'list' }, items) : none,
      h('p', { class: 'section-label', text: 'Notifications' }),
      notes.length ? notes : h('p', { class: 'drawer-intro', text: 'Rien pour l’instant.' }),
    ];
  },

  async memory() {
    const facts = await getJson('/memory-facts');
    const none = empty('memory', 'Rien pour l’instant.', 'Ce que tu me dis d’important est retenu après chaque conversation (2 min d’inactivité).');
    const items = facts.map((fact) => {
      const li = h('li', { class: 'item' },
        h('div', { class: 'item-main' },
          h('span', { class: 'item-title', text: fact.content }),
          h('span', { class: 'item-meta' }, `Retenu le ${dayMonth(new Date(fact.created_at))}`, fact.private && h('span', { class: 'tag', text: 'privé' }))),
        h('button', {
          class: 'btn btn-ghost', type: 'button', 'aria-label': `Oublier « ${fact.content} »`, text: 'Oublier',
          onclick: async () => { await api(`/memory-facts/${fact.id}`, { method: 'DELETE' }); removeItem(li, none); },
        }));
      return li;
    });
    return [
      h('p', { class: 'drawer-intro', text: 'Ce que je retiens de nos conversations, pour mieux t’aider. Tu peux effacer chaque élément.' }),
      items.length ? h('ul', { class: 'list' }, items) : none,
      privacyNotice(),
    ];
  },
};

/** Transparence (RGPD, art. 12-13) : où vont les données et comment les effacer. */
function privacyNotice() {
  const point = (title, text) => h('p', {}, h('strong', { text: title }), ` ${text}`);
  return h('div', { class: 'privacy' },
    h('p', { class: 'section-label', text: 'Tes données' }),
    point('Chez toi.', 'Conversations, mémoire, tâches et journal des actions sont stockés sur ton serveur.'),
    health.cloud_model
      ? point('Pour répondre vite,', `tes messages et le contexte utile partent vers le modèle cloud « ${health.cloud_model} », hors de ton serveur, selon les conditions de son fournisseur. Le contenu d’un mail ouvert ne part jamais : la conversation passe en mode privé, traitée ici.`)
      : point('Tout reste ici :', 'aucun modèle cloud n’est utilisé.'),
    point('Recherches web :', 'elles passent par ton moteur SearXNG, sans compte ni traceur.'),
    voiceEnabled && point('La voix', 'est transcrite et lue sur ton serveur ; l’enregistrement n’est pas conservé.'),
    point('Ton contrôle :', 'efface un élément ici, ou une conversation entière avec ses souvenirs depuis l’Historique. Cette interface ne charge rien d’extérieur et n’utilise ni cookie ni traceur.'));
}

// ---------- Démarrage ----------

function wire() {
  ui.composer.addEventListener('submit', (e) => {
    e.preventDefault();
    if (busy || flags.voice) return;
    const text = ui.input.value;
    ui.input.value = '';
    autosize();
    submit(text);
  });
  ui.input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) { e.preventDefault(); ui.composer.requestSubmit(); }
  });
  ui.input.addEventListener('input', autosize);
  ui.mic.addEventListener('click', () => {
    if (recorder) recorder.stop();
    else if (speech.busy) speech.stop();
    else startRecording();
  });
  $('new-chat').addEventListener('click', () => newConversation());
  document.querySelectorAll('[data-drawer]').forEach((button) => button.addEventListener('click', () => showDrawer(button.dataset.drawer)));
  document.querySelectorAll('[data-close]').forEach((button) => button.addEventListener('click', closeDrawer));
  ui.backdrop.addEventListener('click', closeDrawer);
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape') closeDrawer(); });
}

async function init() {
  wire();
  health = await getJson('/health').catch(() => ({}));
  voiceEnabled = Boolean(health.voice && navigator.mediaDevices?.getUserMedia && window.MediaRecorder);
  ui.mic.hidden = !voiceEnabled;
  const saved = storage.get();
  if (saved) await loadConversation(saved).catch(() => newConversation());
  else newConversation();
}

init();
