var fs = [];
{
  let a = 1;
  fs.push(() => a);
  {
    let a = 2;
    fs.push(() => a);
  }
  a = 3;
}
print(fs[0](), fs[1]());
