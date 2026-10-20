try { throw "e"; } catch (err) { var err = "reassigned"; print(err); }
print(typeof err, err);
