async function foo() {
  return await fetch("http://google.com");
}

let resp = await foo();

console.log(resp);
