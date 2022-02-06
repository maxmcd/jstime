({ randomFloat }) => {
  // TODO: this implementation is incorrect, just assumes u8 even thought many
  // other array types should be supported
  function getRandomValues(abv) {
    var l = abv.length;
    while (l--) {
      abv[l] = Math.floor(randomFloat() * 256);
    }
    return abv;
  }

  globalThis.crypto = { getRandomValues: getRandomValues };
};
