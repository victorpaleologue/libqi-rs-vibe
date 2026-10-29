// Loads a module library the way NAOqi loads the libraries of its autoload.ini: dlopen, then
// `_createModule` declared as libalcommon declares it, with a `boost::shared_ptr<AL::ALBroker>`
// passed by value. The broker is a stand-in class (the module must not use it), with a
// non-trivial destructor checked after the call: the caller owns the temporary.
//
// Usage: fake-naoqi <library> [seconds to stay loaded, default 5]
#include <boost/shared_ptr.hpp>
#include <dlfcn.h>
#include <cstdio>
#include <cstdlib>
#include <unistd.h>

namespace AL {
class ALBroker {
public:
  ALBroker() : magic(0x42adde42) {}
  ~ALBroker() { destroyed = true; }
  unsigned magic;
  static bool destroyed;
};
bool ALBroker::destroyed = false;
}

typedef int (*CreateModule)(boost::shared_ptr<AL::ALBroker>);
typedef int (*CloseModule)();

int main(int argc, char** argv) {
  if (argc < 2) {
    std::fprintf(stderr, "usage: %s <library> [seconds]\n", argv[0]);
    return 2;
  }
  const int seconds = argc > 2 ? std::atoi(argv[2]) : 5;
  // NAOqi loads modules with RTLD_NOW since 1.12.
  void* handle = dlopen(argv[1], RTLD_NOW | RTLD_GLOBAL);
  if (!handle) {
    std::fprintf(stderr, "dlopen failed: %s\n", dlerror());
    return 1;
  }
  CreateModule create = reinterpret_cast<CreateModule>(dlsym(handle, "_createModule"));
  CloseModule close = reinterpret_cast<CloseModule>(dlsym(handle, "_closeModule"));
  if (!create || !close) {
    std::fprintf(stderr, "missing _createModule or _closeModule\n");
    return 1;
  }
  {
    boost::shared_ptr<AL::ALBroker> broker(new AL::ALBroker());
    const int status = create(broker);
    if (status != 0) {
      std::fprintf(stderr, "_createModule returned %d\n", status);
      return 1;
    }
    if (broker.use_count() != 1 || broker->magic != 0x42adde42) {
      std::fprintf(stderr, "the broker was corrupted by _createModule\n");
      return 1;
    }
  }
  if (!AL::ALBroker::destroyed) {
    std::fprintf(stderr, "the broker was leaked\n");
    return 1;
  }
  std::printf("module created\n");
  std::fflush(stdout);
  sleep(seconds);
  const int status = close();
  std::printf("module closed: %d\n", status);
  return status == 0 ? 0 : 1;
}
