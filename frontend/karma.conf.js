// Karma configuration file for ApexStore frontend
module.exports = function (config) {
  config.set({
    basePath: '',
    frameworks: ['jasmine', '@angular-devkit/build-angular'],
    plugins: [
      require('karma-jasmine'),
      require('karma-chrome-launcher'),
      require('karma-jasmine-html-reporter'),
      require('karma-coverage'),
      require('@angular-devkit/build-angular/plugins/karma'),
    ],
    client: {
      jasmine: {
        random: true,
        seed: null, // use constant seed for reproducible runs
      },
    },
    jasmineHtmlReporter: {
      suppressAll: true,
    },
    coverageReporter: {
      dir: require('path').join(__dirname, './coverage/apexstore-frontend'),
      subdir: '.',
      reporters: [
        { type: 'html' },
        { type: 'text-summary' },
        { type: 'lcovonly' },
      ],
    },
    reporters: ['progress', 'kjhtml'],
    // Chrome's sandbox needs a non-root user. GitHub-hosted runners provide one,
    // but a container (the local `node:20` image, or any dockerised CI) usually
    // runs as root, where ChromeHeadless refuses to start:
    //   "Running as root without --no-sandbox is not supported"
    // Use one launcher everywhere so a green local run means a green CI run.
    customLaunchers: {
      ChromeHeadlessNoSandbox: {
        base: 'ChromeHeadless',
        flags: ['--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage'],
      },
    },
    browsers: ['ChromeHeadlessNoSandbox'],
    restartOnFileChange: true,
    singleRun: false,
    failOnEmptyTestSuite: false,
    colors: true,
    logLevel: config.LOG_INFO,
    autoWatch: true,
  });
};
