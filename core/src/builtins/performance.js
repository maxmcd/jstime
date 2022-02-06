"use strict";

// eslint-disable-next-line no-unused-expressions
({ perfNow }) => {
  globalThis.performance = {};
  globalThis.performance.now = perfNow;
};
