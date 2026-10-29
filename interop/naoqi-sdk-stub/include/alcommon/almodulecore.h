// Stub of the NAOqi C++ SDK, for tests only: the callbacks of the loop of the DCM.
#pragma once
#include <boost/function.hpp>
#include <boost/signals2.hpp>
#include <string>

typedef boost::signals2::connection ProcessSignalConnection;

namespace AL {
class ALModuleCore {
 public:
  explicit ALModuleCore(const std::string& name) : fName(name) {}
  virtual ~ALModuleCore() {}
  const std::string& getName() const { return fName; }
  ProcessSignalConnection atPreProcess(boost::function<void()> callback) {
    return fPreProcess.connect(callback);
  }
  ProcessSignalConnection atPostProcess(boost::function<void()> callback) {
    return fPostProcess.connect(callback);
  }
  // Test helpers: one cycle of the loop.
  void stubPreProcess() { fPreProcess(); }
  void stubPostProcess() { fPostProcess(); }

 private:
  std::string fName;
  boost::signals2::signal<void()> fPreProcess;
  boost::signals2::signal<void()> fPostProcess;
};
}  // namespace AL
