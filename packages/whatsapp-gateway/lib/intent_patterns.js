// Pattern packs used by `ownerIntentsRelay()` to recognise when the
// bot's owner is asking the bot to relay a message to a third
// party ("write to Marta that..."), as opposed to talking to the
// bot directly ("tell me a joke").
//
// Patterns are grouped by source language so adding a new locale is
// a file-level change, not a regex-alternation edit. Every pattern
// MUST require an explicit third-party object (preposition + name,
// or a verb form that bakes the indirect object in) — otherwise bare
// imperatives like `"mi dica"` (Italian "tell me", owner→bot) get
// false-positive-matched as a relay intent and re-open the exact
// class of incidents this guard exists to prevent.

const EN_RELAY = [
  // "reply to Marta", "write to the customer"
  'reply\\s+to\\s+\\w+',
  'write\\s+to\\s+\\w+',
  'relay\\s+to\\s+\\w+',
  // "tell <anyone-but-me/us/you> …" — keeps "tell me a joke" out while
  // admitting "tell Alice I am busy", "tell him", "tell them".
  'tell\\s+(?!me\\b|us\\b|you\\b)\\w+',
  // "forward it/this/that/the X/a X to <recipient>" — narrow form so
  // idioms like "look forward to" and "I am forward to hearing from"
  // don't trigger a relay.
  'forward\\s+(?:it|this|that|the\\s+\\w+|a\\s+\\w+)\\s+to\\s+\\w+',
];

// Spanish — every pattern requires explicit recipient so bare
// imperatives like "dime" (tell-me) don't trigger relay mode.
const ES_RELAY = [
  'responde\\s+a\\s+\\w+',
  'escribe\\s+a\\s+\\w+',
  'pregunta\\s+a\\s+\\w+',
  'envía\\s+a\\s+\\w+',
  'envia\\s+a\\s+\\w+',
  'reenvía\\s+a\\s+\\w+',
  'reenvia\\s+a\\s+\\w+',
  'salúdalo', 'saludalo',
  'salúdala', 'saludala',
  'dile\\s+a\\s+\\w+',
  'dígale\\s+a\\s+\\w+',
  'digale\\s+a\\s+\\w+',
];

// French
const FR_RELAY = [
  'réponds\\s+à\\s+\\w+',
  'reponds\\s+à\\s+\\w+',
  'écris\\s+à\\s+\\w+',
  'ecris\\s+à\\s+\\w+',
  'demande\\s+à\\s+\\w+',
  'envoie\\s+à\\s+\\w+',
  'transfère\\s+à\\s+\\w+',
  'transfere\\s+à\\s+\\w+',
  'dis\\s+à\\s+\\w+',
  'dites\\s+à\\s+\\w+',
  'salue\\s+\\w+',
];

// German
const DE_RELAY = [
  'antworte\\s+an\\s+\\w+',
  'schreibe?\\s+an\\s+\\w+',
  'frage\\s+\\w+',
  'sende\\s+an\\s+\\w+',
  'leite\\s+an\\s+\\w+\\s+weiter',
  'sag\\s+\\w+',
  'sage\\s+\\w+',
  'grüße\\s+\\w+',
  'grusse\\s+\\w+',
];

// Portuguese
const PT_RELAY = [
  'responde\\s+a(?:o|à)?\\s+\\w+',
  'responda\\s+a(?:o|à)?\\s+\\w+',
  'escreve\\s+a(?:o|à)?\\s+\\w+',
  'escreva\\s+a(?:o|à)?\\s+\\w+',
  'pergunta\\s+a(?:o|à)?\\s+\\w+',
  'pergunte\\s+a(?:o|à)?\\s+\\w+',
  'envia\\s+a(?:o|à)?\\s+\\w+',
  'envie\\s+a(?:o|à)?\\s+\\w+',
  'encaminhe\\s+a(?:o|à)?\\s+\\w+',
  'encaminha\\s+a(?:o|à)?\\s+\\w+',
  'diga\\s+a(?:o|à)?\\s+\\w+',
  'cumprimenta\\s+\\w+',
];

const IT_RELAY = [
  // "rispondi a Marta", "scrivi al cliente", "chiedi a Luca"
  'rispondi\\s+a(?:l|lla|llo|lle|i|gli)?\\s+\\w+',
  'scrivi\\s+a(?:l|lla|llo|lle|i|gli)?\\s+\\w+',
  'chiedi\\s+a(?:l|lla|llo|lle|i|gli)?\\s+\\w+',
  'manda\\s+a(?:l|lla|llo|lle|i|gli)?\\s+\\w+',
  'inoltra\\s+a(?:l|lla|llo|lle|i|gli)?\\s+\\w+',
  // Verb forms with a baked-in third-person indirect object:
  // "digli" = "tell him", "dille" = "tell her", "scrivigli" = "write to him"
  'digli',
  'dille',
  'scrivigli',
  // "dica a Mario …" — requires the preposition so we don't match
  // "mi dica" (owner addressing the bot in the formal register).
  'dica\\s+a\\s+\\w+',
  // "saluta <name>" — imperative + explicit recipient
  'saluta\\s+\\w+',
];

/**
 * Compile a union regex from the requested language packs.
 *
 * `languages` is a list of two-letter codes (`["en", "it"]`). Unknown
 * codes are silently skipped so a config-side typo can't crash boot.
 */
function compileIntentRegex(languages = ['en']) {
  const packs = {
    en: EN_RELAY,
    it: IT_RELAY,
    es: ES_RELAY,
    fr: FR_RELAY,
    de: DE_RELAY,
    pt: PT_RELAY,
  };
  const patterns = [];
  for (const code of languages) {
    const pack = packs[code.toLowerCase()];
    if (pack) patterns.push(...pack);
  }
  if (patterns.length === 0) {
    // Empty config → never match; safer than always-match.
    return /(?!.*)/;
  }
  return new RegExp('\\b(?:' + patterns.join('|') + ')\\b', 'i');
}

module.exports = {
  EN_RELAY,
  IT_RELAY,
  ES_RELAY,
  FR_RELAY,
  DE_RELAY,
  PT_RELAY,
  compileIntentRegex,
};
