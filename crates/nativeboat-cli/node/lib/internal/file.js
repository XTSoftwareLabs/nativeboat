'use strict';

const { Blob } = require('internal/blob');

class File extends Blob {
  constructor(fileBits, fileName, options) {
    super(fileBits, options);
    this.name = String(fileName);
    const lastModified = options && options.lastModified;
    this.lastModified = typeof lastModified === 'number' ? lastModified : Date.now();
    this.webkitRelativePath = '';
  }

  get [Symbol.toStringTag]() {
    return 'File';
  }
}

module.exports = { File };
