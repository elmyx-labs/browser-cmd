function () {
  // How much of the page's own text is read looking for a wall's words. A wall
  // says so at the top; a long article past that is only cost.
  const TEXT_READ = 3000;
  // What each wall says, as case-insensitive expressions, and what it leaves in
  // the page, as selector lists. A vendor changes both on the same day.
  const BLOCKED = /you have been blocked|error 10[0-9]{2}|access to this page has been denied|access denied|blocked by network security/i;
  const CLOUDFLARE = /just a moment|verify you are human|checking your browser|verifying you are human|attention required/i;
  const DATADOME = /datadome/i;
  const PERIMETERX = /press & hold/i;
  const AWSWAF = /let's confirm you are human/i;
  const CLOUDFLARE_MARKS = '#challenge-running, #challenge-stage, #cf-challenge-running, #turnstile-wrapper';
  const DATADOME_MARKS = 'iframe[src*="captcha-delivery.com"], #datadome-captcha';
  const PERIMETERX_MARKS = '#px-captcha';
  const AWSWAF_MARKS = 'script[src*="captcha.awswaf.com"]';
  // A WAF script on a page with no text yet is a challenge working out its
  // token; the same script on a page with text has already let the visitor in.
  const AWSWAF_CHALLENGE_MARKS = 'script[src*="awswaf.com"]';
  // A widget that has not said which vendor it is yet.
  const WIDGET_MARKS = '.cf-turnstile, .g-recaptcha, .h-captcha, [data-sitekey]';
  // Where a solved challenge puts its token: a filled one means the wall is
  // already behind us, whatever else is still on the page.
  const SOLVED_MARKS = '[name="cf-turnstile-response"], [name="g-recaptcha-response"], [name="h-captcha-response"]';

  const body = document.body ? document.body.innerText.slice(0, TEXT_READ) : '';
  const text = (document.title || '') + ' ' + body;
  const has = (selector) => !!document.querySelector(selector);
  const solved = document.querySelector(SOLVED_MARKS);
  if (solved && solved.value) return 'cleared';
  if (BLOCKED.test(text)) return 'blocked';
  if (CLOUDFLARE.test(text) || has(CLOUDFLARE_MARKS)) return 'cloudflare';
  if (has(DATADOME_MARKS) || DATADOME.test(text)) return 'datadome';
  if (has(PERIMETERX_MARKS) || PERIMETERX.test(text)) return 'perimeterx';
  if (has(AWSWAF_MARKS) || AWSWAF.test(text)) return 'awswaf';
  if (!body.trim() && has(AWSWAF_CHALLENGE_MARKS)) return 'awswaf-challenge';
  if (has(WIDGET_MARKS)) return 'widget';
  return null;
}
