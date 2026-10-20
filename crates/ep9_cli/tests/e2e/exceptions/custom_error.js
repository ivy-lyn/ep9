function MyError(msg) { this.message = msg; this.name = "MyError"; }
MyError.prototype = new Error();
MyError.prototype.constructor = MyError;
try { throw new MyError("custom"); } catch (e) { print(e.name, e.message, e instanceof MyError, e instanceof Error, e.toString()); }
class AppError extends Error { constructor(m) { super(m); this.name = "AppError"; } }
try { throw new AppError("app"); } catch (e) { print(e.name, e.message, e instanceof AppError, e instanceof Error); }
