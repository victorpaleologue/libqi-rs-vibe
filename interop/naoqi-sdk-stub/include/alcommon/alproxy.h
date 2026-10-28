// Stub of the NAOqi C++ SDK, for tests only.
#pragma once
#include <alcommon/almodulecore.h>
#include <boost/shared_ptr.hpp>

namespace AL {
class ALProxy {
 public:
  explicit ALProxy(boost::shared_ptr<ALModuleCore> module) : fModule(module) {}
  boost::shared_ptr<ALModuleCore> getModule() const { return fModule; }

 private:
  boost::shared_ptr<ALModuleCore> fModule;
};
}  // namespace AL
