(function(global) {
  'use strict';

  var SUPPORTED_LANGUAGES = ['en', 'zh-CN'];
  var DEFAULT_LANGUAGE = 'en';
  var STORAGE_KEY = 'librefang-language';

  var currentLanguage = DEFAULT_LANGUAGE;
  var translations = {};
  var loaded = false;

  function detectLanguage() {
    var stored = localStorage.getItem(STORAGE_KEY);
    if (stored && SUPPORTED_LANGUAGES.indexOf(stored) >= 0) return stored;

    var browserLang = navigator.language || navigator.userLanguage || DEFAULT_LANGUAGE;
    if (SUPPORTED_LANGUAGES.indexOf(browserLang) >= 0) return browserLang;

    var langPrefix = browserLang.split('-')[0];
    for (var i = 0; i < SUPPORTED_LANGUAGES.length; i++) {
      if (SUPPORTED_LANGUAGES[i].split('-')[0] === langPrefix) return SUPPORTED_LANGUAGES[i];
    }
    return DEFAULT_LANGUAGE;
  }

  async function loadTranslations(lang) {
    try {
      var response = await fetch('/locales/' + lang + '.json');
      if (!response.ok) throw new Error('HTTP ' + response.status);
      return await response.json();
    } catch (error) {
      console.warn('[LibreFang i18n] failed to load locale', lang, error);
      if (lang !== DEFAULT_LANGUAGE) return loadTranslations(DEFAULT_LANGUAGE);
      return {};
    }
  }

  function getNestedValue(obj, path) {
    return path.split('.').reduce(function(current, key) {
      return current && current[key] !== undefined ? current[key] : null;
    }, obj);
  }

  function replacePlaceholders(str, params) {
    if (!params || typeof str !== 'string') return str;
    return str.replace(/\{(\w+)\}/g, function(match, key) {
      return params[key] !== undefined ? params[key] : match;
    });
  }

  function t(key, params) {
    var value = getNestedValue(translations, key);
    if (value === null) return '[' + key + ']';
    return replacePlaceholders(value, params);
  }

  function updateDOM() {
    document.querySelectorAll('[data-i18n]').forEach(function(element) {
      var translated = t(element.getAttribute('data-i18n'));
      if (!translated.startsWith('[')) element.textContent = translated;
    });

    document.querySelectorAll('[data-i18n-placeholder]').forEach(function(element) {
      var translated = t(element.getAttribute('data-i18n-placeholder'));
      if (!translated.startsWith('[')) element.placeholder = translated;
    });

    document.querySelectorAll('[data-i18n-title]').forEach(function(element) {
      var translated = t(element.getAttribute('data-i18n-title'));
      if (!translated.startsWith('[')) element.title = translated;
    });
  }

  async function init(lang) {
    currentLanguage = lang || detectLanguage();
    translations = await loadTranslations(currentLanguage);
    localStorage.setItem(STORAGE_KEY, currentLanguage);
    loaded = true;
    updateDOM();
    window.dispatchEvent(new CustomEvent('i18n-loaded', { detail: { language: currentLanguage } }));
    return currentLanguage;
  }

  async function setLanguage(lang) {
    if (SUPPORTED_LANGUAGES.indexOf(lang) < 0) return false;
    currentLanguage = lang;
    translations = await loadTranslations(lang);
    localStorage.setItem(STORAGE_KEY, currentLanguage);
    updateDOM();
    window.dispatchEvent(new CustomEvent('i18n-changed', { detail: { language: currentLanguage } }));
    return true;
  }

  global.i18n = {
    init: init,
    t: t,
    setLanguage: setLanguage,
    getLanguage: function() { return currentLanguage; },
    getSupportedLanguages: function() { return SUPPORTED_LANGUAGES.slice(); },
    updateDOM: updateDOM,
    isReady: function() { return loaded; },
    DEFAULT_LANGUAGE: DEFAULT_LANGUAGE,
    SUPPORTED_LANGUAGES: SUPPORTED_LANGUAGES.slice()
  };
})(typeof window !== 'undefined' ? window : this);
