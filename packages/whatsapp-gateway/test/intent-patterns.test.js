'use strict';

const assert = require('node:assert/strict');
const { describe, it } = require('node:test');

const { compileIntentRegex } = require('../lib/intent_patterns');

describe('compileIntentRegex', () => {
  it('matches accented verbs and Unicode recipient names', () => {
    const re = compileIntentRegex(['fr', 'es', 'de']);
    for (const message of [
      'Écris à Élodie demain',
      'Réponds à Łukasz maintenant',
      'Envía a Özlem el informe',
      'Grüße Åsa von mir',
    ]) {
      assert.equal(re.test(message), true, message);
    }
  });

  it('accepts locale identifiers and trims configuration', () => {
    assert.equal(compileIntentRegex([' fr-FR ']).test('Écris à Marie'), true);
    assert.equal(compileIntentRegex(['de_DE']).test('Schreib an Petra'), true);
  });

  it('warns for unknown languages and returns a never-match regex', () => {
    const warnings = [];
    const originalWarn = console.warn;
    console.warn = (message) => warnings.push(message);
    try {
      const re = compileIntentRegex(['Klingon']);
      assert.equal(re.test('tell Alice hello'), false);
    } finally {
      console.warn = originalWarn;
    }
    assert.deepEqual(warnings, ['[gateway] unknown relay intent language ignored: "Klingon"']);
  });

  it('requires the relay imperative at the start of the message', () => {
    const re = compileIntentRegex(['en']);
    assert.equal(re.test('tell Alice I am busy'), true);
    assert.equal(re.test('  write to Bob about this'), true);
    assert.equal(re.test("I can't reply to Marta right now"), false);
    assert.equal(re.test('She said: write to the customer'), false);
    assert.equal(re.test('I will tell Sarah myself later'), false);
  });

  it('rejects German owner-directed pronouns', () => {
    const re = compileIntentRegex(['de']);
    for (const message of ['Sag mir etwas', 'Sag uns etwas', 'Sag euch etwas', 'Sag dir etwas', 'Sag dich an']) {
      assert.equal(re.test(message), false, message);
    }
    assert.equal(re.test('Sag Klaus ich komme später'), true);
  });
});
