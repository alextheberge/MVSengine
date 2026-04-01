module Demo
  SECRET = "hidden"
  private_constant :SECRET

  module_function

  def build(token)
    token
  end
end
