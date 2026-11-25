'use strict';

// Electron's `desktopCapturer` and `contentTracing`.
//
// `desktopCapturer.getSources` lists each display as a screen source, with
// Electron's ids and names. Thumbnails are empty images: a native program
// does not capture the screen to describe it, and windows are not listed
// as sources.
//
// `contentTracing` records no events: there is no Chromium tracing service.
// A recording starts and stops as Electron's does, and `stopRecording`
// writes an empty trace in the Trace Event Format that tracing viewers
// open.

const fs = require('fs');
const os = require('os');
const path = require('path');
const { screen, nativeImage } = require('electron');

function invalidArgument(message) {
  const error = new TypeError(message);
  error.code = 'ERR_INVALID_ARG_TYPE';
  return error;
}

const desktopCapturer = {
  getSources(options) {
    if (options === null || typeof options !== 'object' || !Array.isArray(options.types)) {
      return Promise.reject(invalidArgument('options.types must be an array'));
    }
    const sources = [];
    if (options.types.includes('screen')) {
      const displays = screen.getAllDisplays();
      displays.forEach((display, index) => {
        sources.push({
          id: `screen:${index}:0`,
          name: displays.length === 1 ? 'Entire Screen' : `Screen ${index + 1}`,
          thumbnail: nativeImage.createEmpty(),
          display_id: String(display.id),
          appIcon: null,
        });
      });
    }
    return Promise.resolve(sources);
  },
};

let recording = false;

function traceFile(resultFilePath) {
  if (typeof resultFilePath === 'string' && resultFilePath.length > 0) return resultFilePath;
  const name = `trace-${process.pid}-${Date.now()}.json`;
  return path.join(os.tmpdir(), name);
}

const contentTracing = {
  getCategories() {
    return Promise.resolve([]);
  },
  startRecording(options) {
    if (options !== undefined && (options === null || typeof options !== 'object')) {
      return Promise.reject(invalidArgument('options must be an object'));
    }
    recording = true;
    return Promise.resolve();
  },
  stopRecording(resultFilePath) {
    if (!recording) {
      return Promise.reject(new Error('Failed to stop tracing - no trace in progress'));
    }
    recording = false;
    const file = traceFile(resultFilePath);
    return fs.promises
      .writeFile(file, JSON.stringify({ traceEvents: [], metadata: {} }))
      .then(() => file);
  },
  getTraceBufferUsage() {
    return Promise.resolve({ value: 0, percentage: 0 });
  },
  // Electron 43: heap profiling in the next recording; nothing is sampled.
  enableHeapProfiling() {
    return Promise.resolve();
  },
};

module.exports = { desktopCapturer, contentTracing };
