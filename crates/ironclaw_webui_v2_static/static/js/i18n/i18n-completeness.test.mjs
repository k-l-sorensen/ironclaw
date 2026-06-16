import assert from 'node:assert/strict';
import test from 'node:test';

import { AVAILABLE_LANGUAGES, getRegisteredPacks } from '../lib/i18n.js';
import './ar.js';
import './de.js';
import './en.js';
import './es.js';
import './fr.js';
import './hi.js';
import './ja.js';
import './ko.js';
import './pt-BR.js';
import './uk.js';
import './zh-CN.js';

const BASELINE_MISSING_KEYS = Object.freeze([
  'approval.actionLabel',
  'approval.agentContext',
  'approval.alwaysUnavailable',
  'approval.destinationLabel',
  'approval.notSpecified',
  'approval.nothingSentYet',
  'approval.outboundDataLabel',
  'approval.parametersLabel',
  'approval.shortcutHint',
  'approval.targetLabel',
  'approval.touchesLabel',
  'approval.whatLeavesMachineLabel',
  'authGate.googleHint',
  'chat.addMenuTitle',
  'chat.addToMessage',
  'chat.attachFilesDesc',
  'chat.attachFilesHint',
  'chat.attachmentExtracted',
  'chat.attachmentExtracting',
  'chat.attachmentMetadataOnly',
  'chat.attachmentNoText',
  'chat.briefLabel',
  'chat.briefNeedsSetupDesc',
  'chat.briefNeedsSetupTitle',
  'chat.briefReadyDesc',
  'chat.briefReadyTitle',
  'chat.briefResumeDesc',
  'chat.briefResumeTitle',
  'chat.briefSafetyDesc',
  'chat.briefSafetyTitle',
  'chat.briefWorkspaceDesc',
  'chat.briefWorkspaceTitle',
  'chat.deleteThreadConfirm',
  'chat.dropToAttach',
  'chat.find.earlier',
  'chat.find.next',
  'chat.find.placeholder',
  'chat.find.previous',
  'chat.loadFailed',
  'chat.loadOlder',
  'chat.loadingConversations',
  'chat.modelPopoverActive',
  'chat.modelPopoverAvailable',
  'chat.modelPopoverEmpty',
  'chat.modelPopoverError',
  'chat.modelPopoverManage',
  'chat.modelPopoverManualDesc',
  'chat.modelPopoverManualLabel',
  'chat.modelPopoverManualPlaceholder',
  'chat.modelPopoverManualToggle',
  'chat.modelPopoverNeedsSetupDesc',
  'chat.modelPopoverNoProvider',
  'chat.modelPopoverProvider',
  'chat.modelPopoverTitle',
  'chat.previewModelNote',
  'chat.previewMorePages',
  'chat.previewOmitted',
  'chat.previewRenderFailed',
  'chat.previewRenderingPages',
  'chat.previewSave',
  'chat.previewTitle',
  'chat.previewTruncated',
  'chat.previewUnavailable',
  'chat.resumeDays',
  'chat.resumeHeading',
  'chat.resumeHours',
  'chat.resumeMinutes',
  'chat.searchingOlder',
  'chat.suggestion1Prompt',
  'chat.suggestion2Prompt',
  'chat.suggestion3Prompt',
  'chat.suggestionUse',
  'command.noMatches',
  'command.noMatchesHint',
  'common.retry',
  'extensions.browseKnowledgeApps',
  'extensions.gatewayUnavailable',
  'googleOauth.applied',
  'googleOauth.apply',
  'googleOauth.desc',
  'googleOauth.getClientId',
  'googleOauth.hint',
  'googleOauth.placeholder',
  'googleOauth.restarting',
  'googleOauth.saving',
  'googleOauth.title',
  'job.filterEventsLabel',
  'llm.applyModel',
  'llm.applying',
  'llm.deleteConfirm',
  'llm.deleteTitle',
  'llm.gatewayUnavailable',
  'llm.pickModel',
  'llm.useNearApiKey',
  'nav.work',
  'onboarding.accessLabel',
  'onboarding.accessTitle',
  'onboarding.continue',
  'onboarding.continueGoogle',
  'onboarding.continueWallet',
  'onboarding.firstRun',
  'onboarding.gatewayCheckingDesktop',
  'onboarding.gatewayCheckingWeb',
  'onboarding.gatewayFollowupCopyDesktop',
  'onboarding.gatewayFollowupCopyWeb',
  'onboarding.gatewayPendingCopyDesktop',
  'onboarding.gatewayPendingCopyWeb',
  'onboarding.gatewayReady',
  'onboarding.gatewayUnavailableCopyDesktop',
  'onboarding.gatewayUnavailableCopyWeb',
  'onboarding.gatewayUnavailableDesktop',
  'onboarding.gatewayUnavailableWeb',
  'onboarding.nativeBadge',
  'onboarding.promiseApprovalsBody',
  'onboarding.promiseApprovalsTitle',
  'onboarding.promiseFilesBody',
  'onboarding.promiseFilesTitle',
  'onboarding.promiseModelsBody',
  'onboarding.promiseModelsTitle',
  'onboarding.providerNearaiDescDesktop',
  'onboarding.resumingSession',
  'onboarding.signInGithub',
  'onboarding.staticPreviewUnavailable',
  'onboarding.staticPreviewUnavailableCopy',
  'skills.removeConfirm',
  'skills.removeTitle',
  'tool.riskDelete',
  'tool.riskExport',
  'tool.riskPublish',
  'tool.riskSend',
  'tool.riskTrade',
]);

function sortedUnique(values) {
  return [...new Set(values)].sort();
}

const packs = getRegisteredPacks();
const languageCodes = AVAILABLE_LANGUAGES.map((language) => language.code).sort();
const baselineMissingSet = new Set(BASELINE_MISSING_KEYS);

test('every advertised language has a registered pack', () => {
  assert.deepEqual(Object.keys(packs).sort(), languageCodes);
});

test('i18n baseline is sorted, unique, and references real English keys', () => {
  const englishKeys = Object.keys(packs.en || {}).sort();
  assert.equal(englishKeys.length, 1079);
  assert.deepEqual(BASELINE_MISSING_KEYS, sortedUnique(BASELINE_MISSING_KEYS));

  const unknownBaselineKeys = BASELINE_MISSING_KEYS.filter((key) => !packs.en?.[key]);
  assert.deepEqual(unknownBaselineKeys, []);
});

test('non-English packs do not add missing translation keys beyond the baseline', () => {
  const englishKeys = Object.keys(packs.en || {}).sort();

  for (const lang of languageCodes.filter((code) => code !== 'en')) {
    const translations = packs[lang] || {};
    const translatedKeys = new Set(Object.keys(translations));
    const missing = englishKeys.filter((key) => !translatedKeys.has(key));
    const newMissing = missing.filter((key) => !baselineMissingSet.has(key));
    const extra = Object.keys(translations)
      .filter((key) => !packs.en?.[key])
      .sort();

    assert.deepEqual(newMissing, [], `${lang} has new untranslated keys`);
    assert.ok(
      missing.length <= BASELINE_MISSING_KEYS.length,
      `${lang} has ${missing.length} missing keys; baseline allows ${BASELINE_MISSING_KEYS.length}`
    );
    assert.deepEqual(extra, [], `${lang} has keys that do not exist in English`);
  }
});
