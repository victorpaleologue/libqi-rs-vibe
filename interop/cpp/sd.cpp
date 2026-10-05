// qi-cpp-sd: a standalone qi ServiceDirectory process.
//
//   qi-cpp-sd [--qi-listen-url tcp://127.0.0.1:0]
//
// Prints "LISTENING <url>" (the actual endpoint, with the resolved port) on
// stdout once the service directory accepts connections, then waits for
// SIGINT/SIGTERM.

#include <iostream>
#include <string>
#include <vector>

#include <qi/application.hpp>
#include <qi/session.hpp>
#include "stderr_log.hpp"

namespace
{
  bool takeOption(const std::string& arg, const std::string& name, std::string& out, int& i, int argc, char** argv)
  {
    if (arg == name)
    {
      if (i + 1 >= argc)
        throw std::runtime_error("missing value for " + name);
      out = argv[++i];
      return true;
    }
    if (arg.compare(0, name.size() + 1, name + "=") == 0)
    {
      out = arg.substr(name.size() + 1);
      return true;
    }
    return false;
  }
}

int main(int argc, char** argv)
{
  std::string listenUrl = "tcp://127.0.0.1:0";
  try
  {
    // qi::Application installs the SIGINT/SIGTERM handlers used by run() and
    // leaves unknown options in place.
    qi::Application app(argc, argv);
    interop::routeQiLogsToStderr();

    for (int i = 1; i < argc; ++i)
    {
      const std::string arg = argv[i];
      if (takeOption(arg, "--qi-listen-url", listenUrl, i, argc, argv))
        continue;
      if (arg == "--help" || arg == "-h")
      {
        std::cout << "usage: qi-cpp-sd [--qi-listen-url URL]\n";
        return 0;
      }
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

    app.run(); // returns after SIGINT/SIGTERM (qi::Application::stop)
    sd.close().wait();
    return 0;
  }
  catch (const std::exception& e)
  {
    std::cerr << "qi-cpp-sd: error: " << e.what() << "\n";
    return 1;
  }
}
