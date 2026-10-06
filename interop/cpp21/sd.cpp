// qi-cpp-sd (libqi 2.1): a standalone qi ServiceDirectory process.
//
//   qi-cpp-sd [--qi-listen-url tcp://127.0.0.1:0]
//
// Prints "LISTENING <url>" on stdout once the service directory accepts
// connections, then waits for SIGINT/SIGTERM.

#include <iostream>
#include <stdexcept>
#include <string>
#include <vector>

#include <qi/application.hpp>
#include <qimessaging/session.hpp>
#include "stderr_log.hpp"

int main(int argc, char** argv)
{
  std::string listenUrl = "tcp://127.0.0.1:0";
  try
  {
    qi::Application app(argc, argv);
    interop::routeQiLogsToStderr();
    for (int i = 1; i < argc; ++i)
    {
      const std::string arg = argv[i];
      if (arg == "--qi-listen-url" && i + 1 < argc)
        listenUrl = argv[++i];
      else if (arg.compare(0, 16, "--qi-listen-url=") == 0)
        listenUrl = arg.substr(16);
      else if (arg == "--help" || arg == "-h")
      {
        std::cout << "usage: qi-cpp-sd [--qi-listen-url URL]\n";
        return 0;
      }
      else
        std::cerr << "qi-cpp-sd: ignoring unknown argument " << arg << "\n";
    }

    qi::Session sd;
    sd.listenStandalone(qi::Url(listenUrl)).value();
    const std::vector<qi::Url> endpoints = sd.endpoints();
    if (endpoints.empty())
    {
      std::cerr << "qi-cpp-sd: no endpoint after listenStandalone\n";
      return 1;
    }
    std::cout << "LISTENING " << endpoints.front().str() << std::endl;
    for (size_t i = 1; i < endpoints.size(); ++i)
      std::cerr << "qi-cpp-sd: also listening on " << endpoints[i].str() << "\n";

    app.run();
    sd.close().wait();
    return 0;
  }
  catch (const std::exception& e)
  {
    std::cerr << "qi-cpp-sd: error: " << e.what() << "\n";
    return 1;
  }
}
